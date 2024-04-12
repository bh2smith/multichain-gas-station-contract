use ethers_core::types::U256;
use near_sdk::{
    collections::TreeMap,
    json_types::{U128, U64},
    near_bindgen, require, AccountId, Promise,
};
use near_sdk_contract_tools::{
    owner::{Owner, OwnerExternal},
    pause::Pause,
};

use crate::{
    chain_configuration::{ChainConfiguration, PaymasterConfiguration, ViewPaymasterConfiguration},
    contract_event::TransactionSequenceSigned,
    decode_transaction_request,
    valid_transaction_request::ValidTransactionRequest,
    Contract, ContractExt, Flags, GetForeignChain, PendingTransactionSequence, StorageKey,
};
use lib::{
    asset::AssetId, foreign_address::ForeignAddress, oracle::decode_pyth_price_id, Rejectable,
};

#[allow(clippy::needless_pass_by_value)]
#[near_bindgen]
impl Contract {
    pub fn pause(&mut self) {
        self.assert_owner();
        <Self as Pause>::pause(self);
    }

    pub fn unpause(&mut self) {
        self.assert_owner();
        <Self as Pause>::unpause(self);
    }

    pub fn get_expire_sequence_after_blocks(&self) -> U64 {
        self.expire_sequence_after_blocks.into()
    }

    pub fn set_expire_sequence_after_blocks(&mut self, expire_sequence_after_blocks: U64) {
        self.assert_owner();
        self.expire_sequence_after_blocks = expire_sequence_after_blocks.into();
    }

    pub fn get_signer_contract_id(&self) -> &AccountId {
        &self.signer_contract_id
    }

    pub fn set_signer_contract_id(&mut self, account_id: AccountId) {
        self.assert_owner();
        self.signer_contract_id = account_id;
    }

    pub fn get_flags(&self) -> &Flags {
        &self.flags
    }

    pub fn set_flags(&mut self, flags: Flags) {
        self.assert_owner();
        self.flags = flags;
    }

    pub fn get_receiver_whitelist(&self) -> Vec<ForeignAddress> {
        self.receiver_whitelist.iter().collect()
    }

    pub fn add_to_receiver_whitelist(&mut self, addresses: Vec<ForeignAddress>) {
        self.assert_owner();
        for address in addresses {
            self.receiver_whitelist.insert(&address);
        }
    }

    pub fn remove_from_receiver_whitelist(&mut self, addresses: Vec<ForeignAddress>) {
        self.assert_owner();
        for address in addresses {
            self.receiver_whitelist.remove(&address);
        }
    }

    pub fn clear_receiver_whitelist(&mut self) {
        self.assert_owner();
        self.receiver_whitelist.clear();
    }

    pub fn get_sender_whitelist(&self) -> Vec<AccountId> {
        self.sender_whitelist.iter().collect()
    }

    pub fn add_to_sender_whitelist(&mut self, addresses: Vec<AccountId>) {
        self.assert_owner();
        for address in addresses {
            self.sender_whitelist.insert(&address);
        }
    }

    pub fn remove_from_sender_whitelist(&mut self, addresses: Vec<AccountId>) {
        self.assert_owner();
        for address in addresses {
            self.sender_whitelist.remove(&address);
        }
    }

    pub fn clear_sender_whitelist(&mut self) {
        self.assert_owner();
        self.sender_whitelist.clear();
    }

    pub fn add_foreign_chain(
        &mut self,
        chain_id: U64,
        oracle_asset_id: String,
        transfer_gas: U128,
        fee_rate: (U128, U128),
    ) {
        self.assert_owner();

        self.foreign_chains.insert(
            &chain_id.0,
            &ChainConfiguration {
                next_paymaster: String::new(),
                oracle_asset_id: decode_pyth_price_id(&oracle_asset_id),
                transfer_gas: U256::from(transfer_gas.0).0,
                fee_rate: (fee_rate.0.into(), fee_rate.1.into()),
                paymasters: TreeMap::new(StorageKey::Paymasters(chain_id.0)),
            },
        );
    }

    pub fn set_foreign_chain_oracle_asset_id(&mut self, chain_id: U64, oracle_asset_id: String) {
        self.assert_owner();

        self.with_mut_chain(chain_id.0, |config| {
            config.oracle_asset_id = decode_pyth_price_id(&oracle_asset_id);
        });
    }

    pub fn set_foreign_chain_transfer_gas(&mut self, chain_id: U64, transfer_gas: U128) {
        self.assert_owner();

        self.with_mut_chain(chain_id.0, |config| {
            config.transfer_gas = U256::from(transfer_gas.0).0;
        });
    }

    pub fn remove_foreign_chain(&mut self, chain_id: U64) {
        self.assert_owner();
        if let Some(mut config) = self.foreign_chains.remove(&chain_id.0) {
            config.paymasters.clear();
        }
    }

    pub fn get_foreign_chains(&self) -> Vec<GetForeignChain> {
        self.foreign_chains
            .iter()
            .map(|(chain_id, config)| GetForeignChain {
                chain_id: chain_id.into(),
                oracle_asset_id: near_sdk::bs58::encode(&config.oracle_asset_id).into_string(),
            })
            .collect()
    }

    pub fn add_paymaster(
        &mut self,
        chain_id: U64,
        nonce: u32,
        token_id: String,
        balance: Option<near_sdk::json_types::U128>,
    ) {
        self.assert_owner();

        require!(
            self.paymaster_keys.get(&token_id).is_some(),
            "Token ID is not registered as paymaster",
        );

        self.with_mut_chain(chain_id.0, |chain_config| {
            chain_config.paymasters.insert(
                &token_id,
                &PaymasterConfiguration {
                    nonce,
                    token_id: token_id.clone(),
                    minimum_available_balance: U256::from(balance.map_or(0, |v| v.0)).0,
                },
            );
        });
    }

    pub fn set_paymaster_balance(&mut self, chain_id: U64, token_id: String, balance: U128) {
        #[cfg(not(feature = "debug"))]
        self.assert_owner();

        self.with_mut_chain(chain_id.0, |chain_config| {
            let mut paymaster = chain_config.paymasters.get(&token_id).unwrap_or_reject();
            paymaster.minimum_available_balance = U256::from(balance.0).0;
            chain_config.paymasters.insert(&token_id, &paymaster);
        });
    }

    pub fn increase_paymaster_balance(&mut self, chain_id: U64, token_id: String, balance: U128) {
        #[cfg(not(feature = "debug"))]
        self.assert_owner();

        self.with_mut_chain(chain_id.0, |chain_config| {
            let mut paymaster = chain_config.paymasters.get(&token_id).unwrap_or_reject();
            paymaster.minimum_available_balance =
                (U256(paymaster.minimum_available_balance) + U256::from(balance.0)).0;
            chain_config.paymasters.insert(&token_id, &paymaster);
        });
    }

    pub fn set_paymaster_nonce(&mut self, chain_id: U64, token_id: String, nonce: u32) {
        #[cfg(not(feature = "debug"))]
        self.assert_owner();

        self.with_mut_chain(chain_id.0, |chain_config| {
            let mut paymaster = chain_config.paymasters.get(&token_id).unwrap_or_reject();
            paymaster.nonce = nonce;
            chain_config.paymasters.insert(&token_id, &paymaster);
        });
    }

    /// Note: If a transaction sequence is _already_ pending signatures with
    /// the paymaster getting removed, this method will not prevent those
    /// payloads from getting signed.
    pub fn remove_paymaster(&mut self, chain_id: U64, token_id: String) {
        self.assert_owner();

        self.with_mut_chain(chain_id.0, |chain_config| {
            chain_config.paymasters.remove(&token_id).unwrap_or_reject();
        });
    }

    pub fn get_paymasters(&self, chain_id: U64) -> Vec<ViewPaymasterConfiguration> {
        self.get_chain(chain_id.0)
            .unwrap_or_reject()
            .paymasters
            .iter()
            .map(|(_, p)| ViewPaymasterConfiguration {
                nonce: p.nonce,
                token_id: p.token_id.clone(),
                foreign_address: ForeignAddress::from_raw_public_key(
                    self.paymaster_keys.get(&p.token_id).unwrap_or_reject(),
                ),
                minimum_available_balance: U256(p.minimum_available_balance).as_u128().into(),
            })
            .collect()
    }

    pub fn list_pending_transaction_sequences(
        &self,
        account_id: Option<AccountId>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> std::collections::HashMap<String, PendingTransactionSequence> {
        let mut v: Vec<_> = self.pending_transaction_sequences.iter().collect();

        v.sort_by_cached_key(|&(id, _)| id);

        v.into_iter()
            .filter(|(_, tx)| {
                account_id
                    .as_ref()
                    .map_or(true, |account_id| &tx.created_by_account_id == account_id)
            })
            .skip(offset.map_or(0, |o| o as usize))
            .take(limit.map_or(usize::MAX, |l| l as usize))
            .map(|(id, tx)| (id.to_string(), tx))
            .collect()
    }

    pub fn get_pending_transaction_sequence(&self, id: U64) -> Option<PendingTransactionSequence> {
        self.pending_transaction_sequences.get(&id.0)
    }

    pub fn list_signed_transaction_sequences_after(
        &self,
        block_height: U64,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Vec<TransactionSequenceSigned> {
        self.signed_transaction_sequences
            .iter()
            .skip_while(|s| s.block_height < block_height.0)
            .skip(offset.map_or(0, |o| o as usize))
            .take(limit.map_or(usize::MAX, |l| l as usize))
            .map(|s| s.event)
            .collect()
    }

    pub fn withdraw_collected_fees(
        &mut self,
        asset_id: AssetId,
        amount: Option<U128>,
        receiver_id: Option<AccountId>, // TODO: Pull method instead of push (danger of typos/locked accounts)
    ) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_owner();
        let mut fees = self
            .collected_fees
            .get(&asset_id)
            .expect_or_reject("No fee entry for provided asset ID");

        let amount = amount.unwrap_or(U128(fees.0));

        fees.0 = fees
            .0
            .checked_sub(amount.0)
            .expect_or_reject("Not enough fees to withdraw");

        self.collected_fees.insert(&asset_id, &fees);

        asset_id.transfer(
            receiver_id.unwrap_or_else(|| self.own_get_owner().unwrap()),
            amount,
        )
    }

    pub fn get_collected_fees(&self) -> std::collections::HashMap<AssetId, U128> {
        self.collected_fees.iter().collect()
    }

    pub fn get_foreign_address_for(
        &self,
        account_id: AccountId,
        token_id: String,
    ) -> ForeignAddress {
        ForeignAddress::from_raw_public_key(
            self.user_chain_keys
                .get(&account_id)
                .unwrap_or_reject()
                .get(&token_id)
                .unwrap_or_reject()
                .public_key_bytes,
        )
    }

    pub fn estimate_gas_cost(
        &self,
        transaction_rlp_hex: String,
        local_asset_price: pyth::state::Price,
        foreign_asset_price: pyth::state::Price,
    ) -> U128 {
        let transaction =
            ValidTransactionRequest::try_from(decode_transaction_request(&transaction_rlp_hex))
                .expect_or_reject("Invalid transaction request");

        let foreign_chain_configuration = self.get_chain(transaction.chain_id).unwrap_or_reject();

        let paymaster_transaction_gas = foreign_chain_configuration.transfer_gas();
        let request_tokens_for_gas =
            (transaction.gas() + paymaster_transaction_gas) * transaction.max_fee_per_gas();

        foreign_chain_configuration
            .token_conversion_price(
                request_tokens_for_gas,
                &foreign_asset_price,
                &local_asset_price,
            )
            .into()
    }
}
