mod execution_request;

use std::sync::Arc;
use std::time::Duration;

use bigdecimal::BigDecimal;
use moka::future::Cache;
use tars::orderbook::primitives::MatchedOrderVerbose;
use tars::orderbook::OrderMapper;
use tars::primitives::HTLCAction;

use crate::errors::ExecutorError;
use crate::infrastructure::chain::bitcoin::BitcoinActionExecutor;
use crate::orders::PendingOrdersProvider;

use self::execution_request::OrderExecutionRequest;

pub struct Executor {
    polling_interval_ms: u64,
    chain_identifier: String,
    orders_provider: PendingOrdersProvider,
    order_mapper: OrderMapper,
    action_executor: Arc<BitcoinActionExecutor>,
    cache: Arc<Cache<String, bool>>,
    signer_id: String,
}

impl Executor {
    pub fn new(
        polling_interval_ms: u64,
        chain_identifier: String,
        orders_provider: PendingOrdersProvider,
        order_mapper: OrderMapper,
        action_executor: Arc<BitcoinActionExecutor>,
        cache: Arc<Cache<String, bool>>,
    ) -> Self {
        let signer_id = action_executor.solver_id();
        Self {
            polling_interval_ms,
            chain_identifier,
            orders_provider,
            order_mapper,
            action_executor,
            cache,
            signer_id,
        }
    }

    pub async fn run(&mut self) {
        tracing::info!(
            chain = %self.chain_identifier,
            solver = %self.signer_id,
            "starting bitcoin executor",
        );

        loop {
            let pending_orders = match self
                .orders_provider
                .get_pending_orders(&self.chain_identifier, &self.signer_id)
                .await
            {
                Ok(orders) => orders,
                Err(error) => {
                    tracing::error!(
                        chain = %self.chain_identifier,
                        solver = %self.signer_id,
                        error = %error,
                        "failed to fetch pending orders",
                    );
                    sleep_for_poll_interval(self.polling_interval_ms).await;
                    continue;
                },
            };

            if pending_orders.is_empty() {
                sleep_for_poll_interval(self.polling_interval_ms).await;
                continue;
            }

            let requests = self.prepare_requests(pending_orders).await;
            if requests.is_empty() {
                sleep_for_poll_interval(self.polling_interval_ms).await;
                continue;
            }

            for request in requests {
                match self
                    .action_executor
                    .execute_action(&request.order, &request.action, &request.swap)
                    .await
                {
                    Ok(submitted_requests) => {
                        tracing::info!(
                            chain = %self.chain_identifier,
                            order_id = %request.order_id,
                            action = %request.action,
                            submitted_requests,
                            "submitted bitcoin wallet request(s)",
                        );
                        self.cache.insert(request.cache_key, true).await;
                    },
                    Err(error) => {
                        tracing::error!(
                            chain = %self.chain_identifier,
                            order_id = %request.order_id,
                            action = %request.action,
                            error = %error,
                            "failed to execute bitcoin action",
                        );
                    },
                }
            }

            tokio::time::sleep(Duration::from_millis(self.polling_interval_ms)).await;
        }
    }

    async fn prepare_requests(
        &self,
        pending_orders: Vec<MatchedOrderVerbose>,
    ) -> Vec<OrderExecutionRequest> {
        let latest_block = match self.latest_block_number(&pending_orders).await {
            Ok(block) => block,
            Err(error) => {
                tracing::error!(
                    chain = %self.chain_identifier,
                    error = %error,
                    "failed to fetch bitcoin block height for refund mapping",
                );
                return Vec::new();
            },
        };

        let mut requests = Vec::with_capacity(pending_orders.len());
        for order in pending_orders {
            let order_id = order.create_order.create_id.clone();
            let action_info = match self.order_mapper.map(&order, latest_block.as_ref()).await {
                Ok(action_info) => action_info,
                Err(error) => {
                    tracing::error!(
                        chain = %self.chain_identifier,
                        order_id = %order_id,
                        error = %error,
                        "failed to map order to bitcoin action",
                    );
                    continue;
                },
            };

            if matches!(action_info.action, HTLCAction::NoOp) {
                continue;
            }

            let Some(swap) = action_info.swap else {
                tracing::error!(
                    chain = %self.chain_identifier,
                    order_id = %order_id,
                    action = %action_info.action,
                    "mapped non-noop action without swap payload",
                );
                continue;
            };

            requests.push(OrderExecutionRequest::new(
                order,
                swap,
                action_info.action,
            ));
        }

        self.filter_cached_requests(requests).await
    }

    async fn latest_block_number(
        &self,
        pending_orders: &[MatchedOrderVerbose],
    ) -> Result<Option<BigDecimal>, ExecutorError> {
        if !self
            .order_mapper
            .has_refundable_swaps(&pending_orders.to_vec())
        {
            return Ok(None);
        }

        let height = self.action_executor.current_block_height().await?;
        Ok(Some(BigDecimal::from(height)))
    }

    async fn filter_cached_requests(
        &self,
        requests: Vec<OrderExecutionRequest>,
    ) -> Vec<OrderExecutionRequest> {
        let mut filtered = Vec::with_capacity(requests.len());

        for request in requests {
            if self.cache.get(&request.cache_key).await.is_some() {
                tracing::debug!(
                    chain = %self.chain_identifier,
                    order_id = %request.order_id,
                    action = %request.action,
                    "skipping cached bitcoin action",
                );
                continue;
            }

            filtered.push(request);
        }

        filtered
    }
}

async fn sleep_for_poll_interval(polling_interval_ms: u64) {
    tokio::time::sleep(Duration::from_millis(polling_interval_ms.max(1))).await;
}
