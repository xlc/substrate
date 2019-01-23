// Copyright 2017-2018 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Timestamp manager: provides means to find out the current time.
//!
//! It is expected that the timestamp is set by the validator in the
//! beginning of each block, typically one of the first extrinsics. The timestamp
//! can be set only once per block and must be set each block.
//!
//! Note, that there might be a constraint on how much time must pass
//! before setting the new timestamp, specified by the `tim:block_period`
//! storage entry.
//!
//! # Interaction with the system
//!
//! ## Finalization
//!
//! This module should be hooked up to the finalization routine.

#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
extern crate sr_std as rstd;

#[macro_use]
extern crate srml_support as runtime_support;

extern crate substrate_metadata;
#[macro_use]
extern crate substrate_metadata_derive;

#[cfg(test)]
extern crate substrate_primitives;
#[cfg(test)]
extern crate sr_io as runtime_io;
extern crate sr_primitives as runtime_primitives;
extern crate srml_system as system;
extern crate srml_consensus as consensus;
extern crate parity_codec as codec;
#[macro_use]
extern crate parity_codec_derive;
extern crate substrate_inherents as inherents;

use runtime_support::{StorageValue, Parameter};
use runtime_primitives::traits::{As, SimpleArithmetic, Zero};
use system::ensure_inherent;
use rstd::{result, ops::{Mul, Div}, cmp};
use runtime_support::for_each_tuple;
use inherents::{RuntimeString, InherentIdentifier, ProvideInherent, IsFatalError, InherentData};
#[cfg(feature = "std")]
use inherents::ProvideInherentData;

/// The identifier for the `timestamp` inherent.
pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"timstap0";
/// The type of the inherent.
pub type InherentType = u64;

/// Errors that can occur while checking the timestamp inherent.
#[derive(Encode)]
#[cfg_attr(feature = "std", derive(Debug, Decode))]
pub enum InherentError {
	/// The timestamp is valid in the future.
	/// This is a non-fatal-error and will not stop checking the inherents.
	ValidAtTimestamp(InherentType),
	/// Some other error.
	Other(RuntimeString),
}

impl IsFatalError for InherentError {
	fn is_fatal_error(&self) -> bool {
		match self {
			InherentError::ValidAtTimestamp(_) => false,
			InherentError::Other(_) => true,
		}
	}
}

impl InherentError {
	/// Try to create an instance ouf of the given identifier and data.
	#[cfg(feature = "std")]
	pub fn try_from(id: &InherentIdentifier, data: &[u8]) -> Option<Self> {
		if id == &INHERENT_IDENTIFIER {
			<InherentError as codec::Decode>::decode(&mut &data[..])
		} else {
			None
		}
	}
}

/// Auxiliary trait to extract timestamp inherent data.
pub trait TimestampInherentData {
	/// Get timestamp inherent data.
	fn timestamp_inherent_data(&self) -> Result<InherentType, RuntimeString>;
}

impl TimestampInherentData for InherentData {
	fn timestamp_inherent_data(&self) -> Result<InherentType, RuntimeString> {
		self.get_data(&INHERENT_IDENTIFIER)
			.and_then(|r| r.ok_or_else(|| "Timestamp inherent data not found".into()))
	}
}

#[cfg(feature = "std")]
pub struct InherentDataProvider;

#[cfg(feature = "std")]
impl ProvideInherentData for InherentDataProvider {
	fn inherent_identifier(&self) -> &'static InherentIdentifier {
		&INHERENT_IDENTIFIER
	}

	fn provide_inherent_data(&self, inherent_data: &mut InherentData) -> Result<(), RuntimeString> {
		use std::time::SystemTime;

		let now = SystemTime::now();
		now.duration_since(SystemTime::UNIX_EPOCH)
			.map_err(|_| {
				"Current time is before unix epoch".into()
			}).and_then(|d| {
				let duration: InherentType = d.as_secs();
				inherent_data.put_data(INHERENT_IDENTIFIER, &duration)
			})
	}

	fn error_to_string(&self, error: &[u8]) -> Option<String> {
		InherentError::try_from(&INHERENT_IDENTIFIER, error).map(|e| format!("{:?}", e))
	}
}

/// A trait which is called when the timestamp is set.
pub trait OnTimestampSet<Moment> {
	fn on_timestamp_set(moment: Moment);
}

macro_rules! impl_timestamp_set {
	() => (
		impl<Moment> OnTimestampSet<Moment> for () {
			fn on_timestamp_set(_: Moment) {}
		}
	);

	( $($t:ident)* ) => {
		impl<Moment: Clone, $($t: OnTimestampSet<Moment>),*> OnTimestampSet<Moment> for ($($t,)*) {
			fn on_timestamp_set(moment: Moment) {
				$($t::on_timestamp_set(moment.clone());)*
			}
		}
	}
}

for_each_tuple!(impl_timestamp_set);

pub trait Trait: consensus::Trait + system::Trait {
	/// Type used for expressing timestamp.
	type Moment: Parameter + Default + SimpleArithmetic
		+ Mul<Self::BlockNumber, Output = Self::Moment>
		+ Div<Self::BlockNumber, Output = Self::Moment>;
	/// Something which can be notified when the timestamp is set. Set this to `()` if not needed.
	type OnTimestampSet: OnTimestampSet<Self::Moment>;
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		/// Set the current time.
		///
		/// Extrinsic with this call should be placed at the specific position in the each block
		/// (specified by the Trait::TIMESTAMP_SET_POSITION) typically at the start of the each block.
		/// This call should be invoked exactly once per block. It will panic at the finalization phase,
		/// if this call hasn't been invoked by that time.
		///
		/// The timestamp should be greater than the previous one by the amount specified by `block_period`.
		fn set(origin, #[compact] now: T::Moment) {
			ensure_inherent(origin)?;
			assert!(!<Self as Store>::DidUpdate::exists(), "Timestamp must be updated only once in the block");
			assert!(
				Self::now().is_zero() || now >= Self::now() + Self::block_period(),
				"Timestamp must increment by at least <BlockPeriod> between sequential blocks"
			);
			<Self as Store>::Now::put(now.clone());
			<Self as Store>::DidUpdate::put(true);

			<T::OnTimestampSet as OnTimestampSet<_>>::on_timestamp_set(now);
		}

		fn on_finalise() {
			assert!(<Self as Store>::DidUpdate::take(), "Timestamp must be updated once in the block");
		}
	}
}

decl_storage! {
	trait Store for Module<T: Trait> as Timestamp {
		/// Current time for the current block.
		pub Now get(now) build(|_| T::Moment::sa(0)): T::Moment;
		/// The minimum (and advised) period between blocks.
		pub BlockPeriod get(block_period) config(period): T::Moment = T::Moment::sa(5);

		/// Did the timestamp get updated in this block?
		DidUpdate: bool;
	}
}

impl<T: Trait> Module<T> {

	/// Get the current time for the current block.
	///
	/// NOTE: if this function is called prior the setting the timestamp,
	/// it will return the timestamp of the previous block.
	pub fn get() -> T::Moment {
		Self::now()
	}

	/// Set the timestamp to something in particular. Only used for tests.
	#[cfg(feature = "std")]
	pub fn set_timestamp(now: T::Moment) {
		<Self as Store>::Now::put(now);
	}
}

fn extract_inherent_data(data: &InherentData) -> Result<InherentType, RuntimeString> {
	data.get_data::<InherentType>(&INHERENT_IDENTIFIER)
		.map_err(|_| RuntimeString::from("Invalid timestamp inherent data encoding."))?
		.ok_or_else(|| "Timestamp inherent data is not provided.".into())
}

impl<T: Trait> ProvideInherent for Module<T> {
	type Call = Call<T>;
	type Error = InherentError;
	const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

	fn create_inherent(data: &InherentData) -> Option<Self::Call> {
		let data = extract_inherent_data(data).expect("Gets and decodes timestamp inherent data");

		let next_time = cmp::max(As::sa(data), Self::now() + Self::block_period());
		Some(Call::set(next_time.into()))
	}

	fn check_inherent(call: &Self::Call, data: &InherentData) -> result::Result<(), Self::Error> {
		const MAX_TIMESTAMP_DRIFT: u64 = 60;

		let t = match call {
			Call::set(ref t) => t.clone(),
			_ => return Ok(()),
		}.as_();

		let data = extract_inherent_data(data).map_err(|e| InherentError::Other(e))?;

		let minimum = (Self::now() + Self::block_period()).as_();
		if t > data + MAX_TIMESTAMP_DRIFT {
			Err(InherentError::Other("Timestamp too far in future to accept".into()))
		} else if t < minimum {
			Err(InherentError::ValidAtTimestamp(minimum))
		} else {
			Ok(())
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use runtime_io::{with_externalities, TestExternalities};
	use substrate_primitives::H256;
	use runtime_primitives::BuildStorage;
	use runtime_primitives::traits::{BlakeTwo256, IdentityLookup};
	use runtime_primitives::testing::{Digest, DigestItem, Header, UintAuthorityId};

	impl_outer_origin! {
		pub enum Origin for Test {}
	}

	#[derive(Clone, Eq, PartialEq)]
	pub struct Test;
	impl system::Trait for Test {
		type Origin = Origin;
		type Index = u64;
		type BlockNumber = u64;
		type Hash = H256;
		type Hashing = BlakeTwo256;
		type Digest = Digest;
		type AccountId = u64;
		type Lookup = IdentityLookup<u64>;
		type Header = Header;
		type Event = ();
		type Log = DigestItem;
	}
	impl consensus::Trait for Test {
		type Log = DigestItem;
		type SessionKey = UintAuthorityId;
		type InherentOfflineReport = ();
	}
	impl Trait for Test {
		type Moment = u64;
		type OnTimestampSet = ();
	}
	type Timestamp = Module<Test>;

	#[test]
	fn timestamp_works() {
		let mut t = system::GenesisConfig::<Test>::default().build_storage().unwrap().0;
		t.extend(GenesisConfig::<Test> {
			period: 5,
		}.build_storage().unwrap().0);

		with_externalities(&mut TestExternalities::new(t), || {
			Timestamp::set_timestamp(42);
			assert_ok!(Timestamp::dispatch(Call::set(69), Origin::INHERENT));
			assert_eq!(Timestamp::now(), 69);
		});
	}

	#[test]
	#[should_panic(expected = "Timestamp must be updated only once in the block")]
	fn double_timestamp_should_fail() {
		let mut t = system::GenesisConfig::<Test>::default().build_storage().unwrap().0;
		t.extend(GenesisConfig::<Test> {
			period: 5,
		}.build_storage().unwrap().0);

		with_externalities(&mut TestExternalities::new(t), || {
			Timestamp::set_timestamp(42);
			assert_ok!(Timestamp::dispatch(Call::set(69), Origin::INHERENT));
			let _ = Timestamp::dispatch(Call::set(70), Origin::INHERENT);
		});
	}

	#[test]
	#[should_panic(expected = "Timestamp must increment by at least <BlockPeriod> between sequential blocks")]
	fn block_period_is_enforced() {
		let mut t = system::GenesisConfig::<Test>::default().build_storage().unwrap().0;
		t.extend(GenesisConfig::<Test> {
			period: 5,
		}.build_storage().unwrap().0);

		with_externalities(&mut TestExternalities::new(t), || {
			Timestamp::set_timestamp(42);
			let _ = Timestamp::dispatch(Call::set(46), Origin::INHERENT);
		});
	}
}
