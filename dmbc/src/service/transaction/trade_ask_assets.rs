extern crate exonum;

use exonum::blockchain::Transaction;
use exonum::crypto;
use exonum::crypto::{PublicKey, Signature};
use exonum::messages::Message;
use exonum::storage::Fork;
use serde_json::Value;

use service::CurrencyService;
use service::asset::{Asset, TradeAsset};
use service::transaction::fee::{calculate_fees_for_trade, TxFees};
use service::transaction::utils;

use super::{SERVICE_ID, TX_TRADE_ASK_ASSETS_ID};
use super::schema::transaction_status::{TxStatus, TxStatusSchema};
use super::schema::wallet::WalletSchema;

encoding_struct! {
    struct TradeAskOffer {
        const SIZE = 40;

        field seller: &PublicKey        [00 => 32]
        field assets: Vec<TradeAsset>   [32 => 40]
    }
}

message! {
    struct TxTradeAsk {
        const TYPE = SERVICE_ID;
        const ID = TX_TRADE_ASK_ASSETS_ID;
        const SIZE = 120;

        field buyer:              &PublicKey    [00 => 32]
        field offer:              TradeAskOffer [32 => 40]
        field seed:               u64           [40 => 48]
        field seller_signature:   &Signature    [48 => 112]
        field data_info:          &str          [112 => 120]
    }
}

impl TxTradeAsk {
    pub fn get_fee(&self, view: &mut Fork) -> TxFees {
        calculate_fees_for_trade(view, self.offer().assets())
    }

    fn process(&self, view: &mut Fork) -> TxStatus {
        let (mut platform, mut buyer, mut seller) = WalletSchema::map(view, |mut schema| {
            let platform_key = CurrencyService::get_platfrom_wallet();
            (
                schema.wallet(&platform_key),
                schema.wallet(self.buyer()),
                schema.wallet(self.offer().seller()),
            )
        });

        let fee = self.get_fee(view);

        // pay for tx execution
        if !utils::transfer_coins(view, &mut seller, &mut platform, fee.transaction_fee()) {
            return TxStatus::Fail;
        }

        // initial point for db rollback, in case if transaction has failed
        view.checkpoint();

        // convert trade assets to assets stored on the blockchain
        let trade_assets = self.offer().assets();
        let assets = trade_assets
            .iter()
            .map(|x| x.clone().into())
            .collect::<Vec<Asset>>();
        println!("Buyer {:?} => Seller {:?}", buyer, seller);

        println!("--   Ask/Bid transaction   --");
        println!("Seller's balance before transaction : {:?}", seller);
        println!("Buyer's balance before transaction : {:?}", buyer);

        let offer_price = self.offer().total_price();
        if !utils::transfer_coins(view, &mut buyer, &mut seller, offer_price) {
            view.rollback();
            return TxStatus::Fail;
        }

        if !utils::transfer_assets(view, &mut seller, &mut buyer, &assets) {
            view.rollback();
            return TxStatus::Fail;
        }

        for (mut creator, fee) in fee.assets_fees() {
            println!("\tCreator {:?} will receive {}", creator.pub_key(), fee);
            if !utils::transfer_coins(view, &mut seller, &mut creator, fee) {
                view.rollback();
                return TxStatus::Fail;
            }
        }

        TxStatus::Success
    }
}

impl Transaction for TxTradeAsk {
    fn verify(&self) -> bool {
        if cfg!(fuzzing) {
            return false;
        }
        let keys_ok = *self.buyer() != *self.offer().seller();
        let verify_seller_ok = crypto::verify(
            self.seller_signature(),
            &self.offer().raw,
            self.offer().seller(),
        );
        let verify_buyer_ok = self.verify_signature(self.buyer());

        keys_ok && verify_seller_ok && verify_buyer_ok
    }

    fn execute(&self, view: &mut Fork) {
        let tx_status = self.process(view);
        TxStatusSchema::map(view, |mut schema| {
            schema.set_status(&self.hash(), tx_status)
        });
    }

    fn info(&self) -> Value {
        json!({
            "transaction_data": self,
        })
    }
}

impl TradeAskOffer {
    pub fn total_price(&self) -> u64 {
        self.assets()
            .iter()
            .fold(0, |total, item| total + item.total_price())
    }
}
