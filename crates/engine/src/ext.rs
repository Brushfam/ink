// Copyright 2018-2022 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Provides the same interface as Substrate's FRAME `contract` module.
//!
//! See [the documentation for the `contract` module](https://docs.rs/crate/pallet-contracts)
//! for more information.

use crate::{
    chain_extension::ChainExtensionHandler,
    database::Database,
    exec_context::ExecContext,
    test_api::{
        DebugInfo,
        EmittedEvent,
    },
    types::{
        AccountId,
        Balance,
        BlockTimestamp,
    },
};
use scale::Encode;
use std::{
    cell::RefCell,
    collections::HashMap,
    panic::panic_any,
    rc::Rc,
};

type Result = core::result::Result<(), Error>;

macro_rules! define_error_codes {
    (
        $(
            $( #[$attr:meta] )*
            $name:ident = $discr:literal,
        )*
    ) => {
        /// Every error that can be returned to a contract when it calls any of the host functions.
        #[cfg_attr(test, derive(PartialEq, Eq))]
        #[derive(Debug)]
        #[repr(u32)]
        pub enum Error {
            $(
                $( #[$attr] )*
                $name = $discr,
            )*
            /// Returns if an unknown error was received from the host module.
            Unknown,
        }

        impl From<ReturnCode> for Result {
            #[inline]
            fn from(return_code: ReturnCode) -> Self {
                match return_code.0 {
                    0 => Ok(()),
                    $(
                        $discr => Err(Error::$name),
                    )*
                    _ => Err(Error::Unknown),
                }
            }
        }
    };
}
define_error_codes! {
    /// The called function trapped and has its state changes reverted.
    /// In this case no output buffer is returned.
    /// Can only be returned from `call` and `instantiate`.
    CalleeTrapped = 1,
    /// The called function ran to completion but decided to revert its state.
    /// An output buffer is returned when one was supplied.
    /// Can only be returned from `call` and `instantiate`.
    CalleeReverted = 2,
    /// The passed key does not exist in storage.
    KeyNotFound = 3,
    /// Deprecated and no longer returned: There is only the minimum balance.
    _BelowSubsistenceThreshold = 4,
    /// Transfer failed for other not further specified reason. Most probably
    /// reserved or locked balance of the sender that was preventing the transfer.
    TransferFailed = 5,
    /// Deprecated and no longer returned: Endowment is no longer required.
    _EndowmentTooLow = 6,
    /// No code could be found at the supplied code hash.
    CodeNotFound = 7,
    /// The account that was called is no contract.
    NotCallable = 8,
    /// The call to `debug_message` had no effect because debug message
    /// recording was disabled.
    LoggingDisabled = 9,
    /// ECDSA public key recovery failed. Most probably wrong recovery id or signature.
    EcdsaRecoveryFailed = 11,
}

/// The raw return code returned by the host side.
#[repr(transparent)]
pub struct ReturnCode(u32);

impl ReturnCode {
    /// Returns the raw underlying `u32` representation.
    pub fn into_u32(self) -> u32 {
        self.0
    }
}

#[derive(Default)]
pub struct ContractStorage {
    pub instantiated: HashMap<Vec<u8>, Vec<u8>>,
    pub entrance_count: HashMap<Vec<u8>, u32>,
    pub allow_reentry: HashMap<Vec<u8>, bool>,
    pub deployed: HashMap<Vec<u8>, Contract>,
}

impl ContractStorage {
    pub fn get_entrance_count(&self, callee: Vec<u8>) -> u32 {
        *self.entrance_count.get(&callee).unwrap_or(&0)
    }

    pub fn get_allow_reentry(&self, callee: Vec<u8>) -> bool {
        *self.allow_reentry.get(&callee).unwrap_or(&false)
    }

    pub fn set_allow_reentry(&mut self, callee: Vec<u8>, allow: bool) {
        if allow {
            self.allow_reentry.insert(callee, allow);
        } else {
            self.allow_reentry.remove(&callee);
        }
    }

    pub fn increase_entrance_count(&mut self, callee: Vec<u8>) -> Result {
        let entrance_count = self
            .entrance_count
            .get(&callee)
            .map_or(1, |count| count + 1);
        self.entrance_count.insert(callee, entrance_count);

        Ok(())
    }

    pub fn decrease_entrance_count(&mut self, callee: Vec<u8>) -> Result {
        let entrance_count = self.entrance_count.get(&callee).map_or_else(
            || Err(Error::CalleeTrapped),
            |count| {
                if *count == 0 {
                    Err(Error::CalleeTrapped)
                } else {
                    Ok(count - 1)
                }
            },
        )?;

        self.entrance_count.insert(callee, entrance_count);
        Ok(())
    }
}

pub struct Contract {
    pub deploy: fn(),
    pub call: fn(),
}

/// The off-chain engine.
#[derive(Clone)]
pub struct Engine {
    /// The environment database.
    pub database: Rc<RefCell<Database>>,
    /// The current execution context.
    pub exec_context: Rc<RefCell<ExecContext>>,
    /// Recorder for relevant interactions with the engine.
    /// This is specifically about debug info. This info is
    /// not available in the `contracts` pallet.
    pub(crate) debug_info: Rc<RefCell<DebugInfo>>,
    /// The chain specification.
    pub chain_spec: Rc<RefCell<ChainSpec>>,
    /// Handler for registered chain extensions.
    pub chain_extension_handler: Rc<RefCell<ChainExtensionHandler>>,
    /// Contracts' store.
    pub contracts: Rc<RefCell<ContractStorage>>,
}

/// The chain specification.
pub struct ChainSpec {
    /// The current gas price.
    pub gas_price: Balance,
    /// The minimum value an account of the chain must have
    /// (i.e. the chain's existential deposit).
    pub minimum_balance: Balance,
    /// The targeted block time.
    pub block_time: BlockTimestamp,
}

/// The default values for the chain specification are:
///
///   * `gas_price`: 100
///   * `minimum_balance`: 42
///   * `block_time`: 6
///
/// There is no particular reason behind choosing them this way.
impl Default for ChainSpec {
    fn default() -> Self {
        Self {
            gas_price: 100,
            minimum_balance: 1000000,
            block_time: 6,
        }
    }
}

impl Engine {
    // Creates a new `Engine instance.
    pub fn new() -> Self {
        Self {
            database: Rc::new(RefCell::new(Database::new())),
            exec_context: Rc::new(RefCell::new(ExecContext::new())),
            debug_info: Rc::new(RefCell::new(DebugInfo::new())),
            chain_spec: Rc::new(RefCell::new(ChainSpec::default())),
            chain_extension_handler: Rc::new(RefCell::new(ChainExtensionHandler::new())),
            contracts: Rc::new(RefCell::new(ContractStorage::default())),
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Transfers value from the contract to the destination account.
    pub fn transfer(&mut self, account_id: &[u8], mut value: &[u8]) -> Result {
        // Note that a transfer of `0` is allowed here
        let increment = <u128 as scale::Decode>::decode(&mut value)
            .map_err(|_| Error::TransferFailed)?;

        let dest = account_id.to_vec();
        // Note that the destination account does not have to exist
        let dest_old_balance = self.get_balance(dest.clone()).unwrap_or_default();

        let contract = self.get_callee();
        let contract_old_balance = self
            .get_balance(contract.clone())
            .map_err(|_| Error::TransferFailed)?;

        self.database
            .borrow_mut()
            .set_balance(&contract, contract_old_balance - increment);
        self.database
            .borrow_mut()
            .set_balance(&dest, dest_old_balance + increment);
        Ok(())
    }

    /// Deposits an event identified by the supplied topics and data.
    pub fn deposit_event(&mut self, topics: &[u8], data: &[u8]) {
        // The first byte contains the number of topics in the slice
        let topics_count: scale::Compact<u32> = scale::Decode::decode(&mut &topics[0..1])
            .unwrap_or_else(|err| panic!("decoding number of topics failed: {}", err));
        let topics_count = topics_count.0 as usize;

        let topics_vec = if topics_count > 0 {
            // The rest of the slice contains the topics
            let topics = &topics[1..];
            let bytes_per_topic = topics.len() / topics_count;
            let topics_vec: Vec<Vec<u8>> = topics
                .chunks(bytes_per_topic)
                .map(|chunk| chunk.to_vec())
                .collect();
            assert_eq!(topics_count, topics_vec.len());
            topics_vec
        } else {
            Vec::new()
        };

        self.debug_info.borrow_mut().record_event(EmittedEvent {
            topics: topics_vec,
            data: data.to_vec(),
        });
    }

    /// Writes the encoded value into the storage at the given key.
    /// Returns the size of the previously stored value at the key if any.
    pub fn set_storage(&mut self, key: &[u8], encoded_value: &[u8]) -> Option<u32> {
        let callee = self.get_callee();
        let account_id = AccountId::from_bytes(&callee[..]);

        self.debug_info.borrow_mut().inc_writes(account_id.clone());
        self.debug_info
            .borrow_mut()
            .record_cell_for_account(account_id, key.to_vec());

        self.database
            .borrow_mut()
            .insert_into_contract_storage(&callee, key, encoded_value.to_vec())
            .map(|v| <u32>::try_from(v.len()).expect("usize to u32 conversion failed"))
    }

    /// Returns the decoded contract storage at the key if any.
    pub fn get_storage(&mut self, key: &[u8], output: &mut &mut [u8]) -> Result {
        let callee = self.get_callee();
        let account_id = AccountId::from_bytes(&callee[..]);

        self.debug_info.borrow_mut().inc_reads(account_id);
        match self
            .database
            .borrow_mut()
            .get_from_contract_storage(&callee, key)
        {
            Some(val) => {
                set_output(output, val);
                Ok(())
            }
            None => Err(Error::KeyNotFound),
        }
    }

    /// Removes the storage entries at the given key,
    /// returning previously stored value at the key if any.
    pub fn take_storage(&mut self, key: &[u8], output: &mut &mut [u8]) -> Result {
        let callee = self.get_callee();
        let account_id = AccountId::from_bytes(&callee[..]);

        self.debug_info.borrow_mut().inc_writes(account_id);
        match self
            .database
            .borrow_mut()
            .remove_contract_storage(&callee, key)
        {
            Some(val) => {
                set_output(output, &val);
                Ok(())
            }
            None => Err(Error::KeyNotFound),
        }
    }

    /// Returns the size of the value stored in the contract storage at the key if any.
    pub fn contains_storage(&mut self, key: &[u8]) -> Option<u32> {
        let callee = self.get_callee();
        let account_id = AccountId::from_bytes(&callee[..]);

        self.debug_info.borrow_mut().inc_reads(account_id);
        self.database
            .borrow_mut()
            .get_from_contract_storage(&callee, key)
            .map(|val| val.len() as u32)
    }

    /// Removes the storage entries at the given key.
    /// Returns the size of the previously stored value at the key if any.
    pub fn clear_storage(&mut self, key: &[u8]) -> Option<u32> {
        let callee = self.get_callee();
        let account_id = AccountId::from_bytes(&callee[..]);
        self.debug_info.borrow_mut().inc_writes(account_id.clone());
        let _ = self
            .debug_info
            .borrow_mut()
            .remove_cell_for_account(account_id, key.to_vec());
        self.database
            .borrow_mut()
            .remove_contract_storage(&callee, key)
            .map(|val| val.len() as u32)
    }

    /// Remove the calling account and transfer remaining balance.
    ///
    /// This function never returns. Either the termination was successful and the
    /// execution of the destroyed contract is halted. Or it failed during the
    /// termination which is considered fatal.
    pub fn terminate(&mut self, beneficiary: &[u8]) -> ! {
        // Send the remaining balance to the beneficiary
        let contract = self.get_callee();
        let all = self
            .get_balance(contract)
            .unwrap_or_else(|err| panic!("could not get balance: {:?}", err));
        let value = &scale::Encode::encode(&all)[..];
        self.transfer(beneficiary, value)
            .unwrap_or_else(|err| panic!("transfer did not work: {:?}", err));

        // Encode the result of the termination and panic with it.
        // This enables testing for the proper result and makes sure this
        // method returns `Never`.
        let res = (all, beneficiary.to_vec());
        panic_any(scale::Encode::encode(&res));
    }

    /// Returns the address of the caller.
    pub fn caller(&self, output: &mut &mut [u8]) {
        let caller = self
            .exec_context
            .borrow()
            .caller
            .as_ref()
            .expect("no caller has been set")
            .clone();
        set_output(output, caller.as_bytes());
    }

    /// Returns the balance of the executed contract.
    pub fn balance(&self, output: &mut &mut [u8]) {
        let contract = self
            .exec_context
            .borrow()
            .callee
            .as_ref()
            .expect("no callee has been set")
            .clone();

        let balance_in_storage = self
            .database
            .borrow()
            .get_balance(contract.as_bytes())
            .expect("currently executing contract must exist");
        let balance = scale::Encode::encode(&balance_in_storage);
        set_output(output, &balance[..])
    }

    /// Returns the transferred value for the called contract.
    pub fn value_transferred(&self, output: &mut &mut [u8]) {
        let value_transferred: Vec<u8> =
            scale::Encode::encode(&self.exec_context.borrow().value_transferred);
        set_output(output, &value_transferred[..])
    }

    /// Returns the address of the executed contract.
    pub fn address(&self, output: &mut &mut [u8]) {
        let callee = self
            .exec_context
            .borrow()
            .callee
            .as_ref()
            .expect("no callee has been set")
            .clone();
        set_output(output, callee.as_bytes())
    }

    /// Records the given debug message and appends to stdout.
    pub fn debug_message(&mut self, message: &str) {
        self.debug_info
            .borrow_mut()
            .record_debug_message(String::from(message));
        print!("{}", message);
    }

    /// Conduct the BLAKE-2 256-bit hash and place the result into `output`.
    pub fn hash_blake2_256(input: &[u8], output: &mut [u8; 32]) {
        super::hashing::blake2b_256(input, output);
    }

    /// Conduct the BLAKE-2 128-bit hash and place the result into `output`.
    pub fn hash_blake2_128(input: &[u8], output: &mut [u8; 16]) {
        super::hashing::blake2b_128(input, output);
    }

    /// Conduct the SHA-2 256-bit hash and place the result into `output`.
    pub fn hash_sha2_256(input: &[u8], output: &mut [u8; 32]) {
        super::hashing::sha2_256(input, output);
    }

    /// Conduct the KECCAK 256-bit hash and place the result into `output`.
    pub fn hash_keccak_256(input: &[u8], output: &mut [u8; 32]) {
        super::hashing::keccak_256(input, output);
    }

    /// Returns the current block number.
    pub fn block_number(&self, output: &mut &mut [u8]) {
        let block_number: Vec<u8> =
            scale::Encode::encode(&self.exec_context.borrow().block_number);
        set_output(output, &block_number[..])
    }

    /// Returns the timestamp of the current block.
    pub fn block_timestamp(&self, output: &mut &mut [u8]) {
        let block_timestamp: Vec<u8> =
            scale::Encode::encode(&self.exec_context.borrow().block_timestamp);
        set_output(output, &block_timestamp[..])
    }

    pub fn gas_left(&self, _output: &mut &mut [u8]) {
        unimplemented!("off-chain environment does not yet support `gas_left`");
    }

    /// Returns the minimum balance that is required for creating an account
    /// (i.e. the chain's existential deposit).
    pub fn minimum_balance(&self, output: &mut &mut [u8]) {
        let minimum_balance: Vec<u8> =
            scale::Encode::encode(&self.chain_spec.borrow().minimum_balance);
        set_output(output, &minimum_balance[..])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn instantiate(
        &mut self,
        _code_hash: &[u8],
        _gas_limit: u64,
        _endowment: &[u8],
        _input: &[u8],
        _out_address: &mut &mut [u8],
        _out_return_value: &mut &mut [u8],
        _salt: &[u8],
    ) -> Result {
        unimplemented!("off-chain environment does not yet support `instantiate`");
    }

    pub fn call(
        &mut self,
        _callee: &[u8],
        _gas_limit: u64,
        _value: &[u8],
        _input: &[u8],
        _output: &mut &mut [u8],
    ) -> Result {
        unimplemented!("off-chain environment does not yet support `call`");
    }

    /// Emulates gas price calculation.
    pub fn weight_to_fee(&self, gas: u64, output: &mut &mut [u8]) {
        let fee = self
            .chain_spec
            .borrow()
            .gas_price
            .saturating_mul(gas.into());
        let fee: Vec<u8> = scale::Encode::encode(&fee);
        set_output(output, &fee[..])
    }

    /// Calls the chain extension method registered at `func_id` with `input`.
    pub fn call_chain_extension(
        &mut self,
        func_id: u32,
        input: &[u8],
        output: &mut &mut [u8],
    ) {
        let encoded_input = input.encode();
        let mut chain_extension_handler = self.chain_extension_handler.borrow_mut();
        let (status_code, out) = chain_extension_handler
            .eval(func_id, &encoded_input)
            .unwrap_or_else(|error| {
                panic!(
                    "Encountered unexpected missing chain extension method: {:?}",
                    error
                );
            });
        let res = (status_code, out);
        let decoded: Vec<u8> = scale::Encode::encode(&res);
        set_output(output, &decoded[..])
    }

    /// Recovers the compressed ECDSA public key for given `signature` and `message_hash`,
    /// and stores the result in `output`.
    pub fn ecdsa_recover(
        &mut self,
        signature: &[u8; 65],
        message_hash: &[u8; 32],
        output: &mut [u8; 33],
    ) -> Result {
        use secp256k1::{
            ecdsa::{
                RecoverableSignature,
                RecoveryId,
            },
            Message,
            SECP256K1,
        };

        // In most implementations, the v is just 0 or 1 internally, but 27 was added
        // as an arbitrary number for signing Bitcoin messages and Ethereum adopted that as well.
        let recovery_byte = if signature[64] > 26 {
            signature[64] - 27
        } else {
            signature[64]
        };

        let recovery_id = RecoveryId::from_i32(recovery_byte as i32)
            .unwrap_or_else(|error| panic!("Unable to parse the recovery id: {}", error));

        let message = Message::from_slice(message_hash).unwrap_or_else(|error| {
            panic!("Unable to create the message from hash: {}", error)
        });
        let signature =
            RecoverableSignature::from_compact(&signature[0..64], recovery_id)
                .unwrap_or_else(|error| {
                    panic!("Unable to parse the signature: {}", error)
                });

        let pub_key = SECP256K1.recover_ecdsa(&message, &signature);
        match pub_key {
            Ok(pub_key) => {
                *output = pub_key.serialize();
                Ok(())
            }
            Err(_) => Err(Error::EcdsaRecoveryFailed),
        }
    }

    /// Register the contract into the local storage.
    pub fn register_contract(
        &mut self,
        hash: &[u8],
        deploy: fn(),
        call: fn(),
    ) -> Option<Contract> {
        self.contracts
            .borrow_mut()
            .deployed
            .insert(hash.to_vec(), Contract { deploy, call })
    }

    /// Apply call flags for the call and return the input that might be changed
    pub fn apply_code_flags_before_call(
        &mut self,
        caller: Option<AccountId>,
        callee: Vec<u8>,
        call_flags: u32,
        input: Vec<u8>,
    ) -> core::result::Result<Vec<u8>, Error> {
        let forward_input = (call_flags & 1) != 0;
        let clone_input = ((call_flags & 2) >> 1) != 0;
        let allow_reentry = ((call_flags & 8) >> 3) != 0;

        // Allow/deny reentrancy to the caller
        if let Some(caller) = caller {
            self.contracts
                .borrow_mut()
                .set_allow_reentry(caller.as_bytes().to_vec(), allow_reentry);
        }

        // Check if reentrance that is not allowed is encountered
        if !self.contracts.borrow().get_allow_reentry(callee.clone())
            && self.contracts.borrow().get_entrance_count(callee.clone()) > 0
        {
            return Err(Error::CalleeTrapped)
        }

        self.contracts
            .borrow_mut()
            .increase_entrance_count(callee)?;

        let new_input = if forward_input {
            let previous_input = self.exec_context.borrow().input.clone();

            // delete the input because we will forward it
            self.exec_context.borrow_mut().input.clear();

            previous_input
        } else if clone_input {
            self.exec_context.borrow().input.clone()
        } else {
            input
        };

        Ok(new_input)
    }

    /// Apply call flags after the call
    pub fn apply_code_flags_after_call(
        &mut self,
        caller: Option<AccountId>,
        callee: Vec<u8>,
        call_flags: u32,
        output: Vec<u8>,
    ) -> core::result::Result<(), Error> {
        let tail_call = ((call_flags & 4) >> 2) != 0;

        if tail_call {
            self.exec_context.borrow_mut().output = output;
        }

        self.contracts
            .borrow_mut()
            .decrease_entrance_count(callee)?;

        if let Some(caller) = caller {
            self.contracts
                .borrow_mut()
                .allow_reentry
                .remove(&caller.as_bytes().to_vec());
        }
        Ok(())
    }
}

/// Copies the `slice` into `output`.
///
/// Panics if the slice is too large and does not fit.
fn set_output(output: &mut &mut [u8], slice: &[u8]) {
    assert!(
        slice.len() <= output.len(),
        "the output buffer is too small! the decoded storage is of size {} bytes, \
        but the output buffer has only room for {}.",
        slice.len(),
        output.len(),
    );
    output[..slice.len()].copy_from_slice(slice);
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    pub fn contract_storage_works() {
        let mut storage = ContractStorage::default();

        let account = [0u8; 32].to_vec();

        assert!(!storage.get_allow_reentry(account.clone()));
        storage.set_allow_reentry(account.clone(), true);
        assert!(storage.get_allow_reentry(account.clone()));

        assert_eq!(storage.increase_entrance_count(account.clone()), Ok(()));
        assert_eq!(storage.get_entrance_count(account.clone()), 1);
        assert_eq!(storage.decrease_entrance_count(account.clone()), Ok(()));
        assert_eq!(storage.get_entrance_count(account), 0);
    }

    #[test]
    pub fn decrease_entrance_count_fails() {
        let mut storage = ContractStorage::default();

        let account = [0u8; 32].to_vec();

        assert_eq!(
            storage.decrease_entrance_count(account),
            Err(Error::CalleeTrapped)
        );
    }
}
