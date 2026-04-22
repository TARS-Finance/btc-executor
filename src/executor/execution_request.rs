use tars::orderbook::primitives::{MatchedOrderVerbose, SingleSwap};
use tars::primitives::HTLCAction;

#[derive(Clone, Debug)]
pub struct OrderExecutionRequest {
    pub cache_key: String,
    pub order_id: String,
    pub action: HTLCAction,
    pub order: MatchedOrderVerbose,
    pub swap: SingleSwap,
}

impl OrderExecutionRequest {
    pub fn new(order: MatchedOrderVerbose, swap: SingleSwap, action: HTLCAction) -> Self {
        let order_id = order.create_order.create_id.clone();
        let cache_key = format!("{order_id}:{action}");

        Self {
            cache_key,
            order_id,
            action,
            order,
            swap,
        }
    }
}
