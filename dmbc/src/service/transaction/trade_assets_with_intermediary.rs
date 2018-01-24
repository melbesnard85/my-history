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
use service::transaction::utils::Intermediary;

use super::{SERVICE_ID, TX_TRADE_ASSETS_WITH_INTERMEDIARY_ID};
use super::schema::transaction_status::{TxStatus, TxStatusSchema};
use super::schema::wallet::WalletSchema;

encoding_struct! {
    struct TradeOfferWithIntermediary {
        const SIZE = 80;

        field intermediary: Intermediary [00 => 08]
        field buyer: &PublicKey          [08 => 40]
        field seller: &PublicKey         [40 => 72]
        field assets: Vec<TradeAsset>    [72 => 80]
    }
}

message! {
    struct TxTradeWithIntermediary {
        const TYPE = SERVICE_ID;
        const ID = TX_TRADE_ASSETS_WITH_INTERMEDIARY_ID;
        const SIZE = 152;

        field offer:              TradeOfferWithIntermediary [00 => 08]
        field seed:               u64                        [08 => 16]
        field seller_signature:   &Signature                 [16 => 80]
        field intermediary_signature: &Signature             [80 => 144]
        field data_info:          &str                       [144 => 152]
    }
}

impl TradeOfferWithIntermediary {
    pub fn total_price(&self) -> u64 {
        self.assets()
            .iter()
            .fold(0, |total, item| total + item.total_price())
    }
}

impl TxTradeWithIntermediary {
    pub fn get_offer_raw(&self) -> Vec<u8> {
        self.offer().raw
    }

    pub fn get_fee(&self, view: &mut Fork) -> TxFees {
        calculate_fees_for_trade(view, self.offer().assets())
    }

    fn process(&self, view: &mut Fork) -> TxStatus {
        let (mut platform, mut buyer, mut seller, mut intermediary) =
            WalletSchema::map(view, |mut schema| {
                let platform_key = CurrencyService::get_platfrom_wallet();
                (
                    schema.wallet(&platform_key),
                    schema.wallet(self.offer().buyer()),
                    schema.wallet(self.offer().seller()),
                    schema.wallet(self.offer().intermediary().wallet()),
                )
            });

        let fee = self.get_fee(view);

        // pay for tx execution
        if !utils::pay(view, &mut seller, &mut platform, fee.transaction_fee()) {
            return TxStatus::Fail;
        }

        // initial point for db rollback, in case if transaction has failed
        view.checkpoint();

        // pay commison for the transaction to intermediary
        if !utils::pay(
            view,
            &mut seller,
            &mut intermediary,
            self.offer().intermediary().commision(),
        ) {
            view.rollback();
            return TxStatus::Fail;
        }

        // convert trade assets to assets stored on the blockchain
        let trade_assets = self.offer().assets();
        let assets = trade_assets
            .iter()
            .map(|x| x.clone().into())
            .collect::<Vec<Asset>>();
        println!("Buyer {:?} => Seller {:?}", buyer, seller);

        println!("--   Trade transaction   --");
        println!("Seller's balance before transaction : {:?}", seller);
        println!("Buyer's balance before transaction : {:?}", buyer);

        let offer_price = self.offer().total_price();
        if !utils::pay(view, &mut buyer, &mut seller, offer_price) {
            view.rollback();
            return TxStatus::Fail;
        }

        if !utils::transfer_assets(view, &mut seller, &mut buyer, &assets) {
            view.rollback();
            return TxStatus::Fail;
        }

        for (mut creator, fee) in fee.assets_fees() {
            println!("\tCreator {:?} will receive {}", creator.pub_key(), fee);
            if !utils::pay(view, &mut seller, &mut creator, fee) {
                view.rollback();
                return TxStatus::Fail;
            }
        }

        TxStatus::Success
    }
}

impl Transaction for TxTradeWithIntermediary {
    fn verify(&self) -> bool {
        if cfg!(fuzzing) {
            return false;
        }

        let mut keys_ok = *self.offer().seller() != *self.offer().buyer();
        keys_ok &= *self.offer().seller() != *self.offer().intermediary().wallet();
        keys_ok &= *self.offer().buyer() != *self.offer().intermediary().wallet();

        let verify_seller_ok = crypto::verify(
            self.seller_signature(),
            &self.offer().raw,
            self.offer().seller(),
        );

        // not sure if this is ok
        let verify_buyer_ok = self.verify_signature(self.offer().buyer());

        let verify_intermediary_ok = crypto::verify(
            self.intermediary_signature(),
            &self.offer().raw,
            self.offer().intermediary().wallet(),
        );

        keys_ok && verify_buyer_ok && verify_seller_ok && verify_intermediary_ok
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
