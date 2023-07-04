use std::collections::HashMap;
use std::sync::Arc;

use ethers::prelude::*;
use futures::future::join_all;
use polars::prelude::*;
use tokio::sync::Semaphore;

use crate::chunks::ChunkAgg;
use crate::types::BlockChunk;
use crate::types::CollectError;
use crate::types::ColumnType;
use crate::types::Dataset;
use crate::types::Datatype;
use crate::types::FreezeOpts;
use crate::types::RateLimiter;
use crate::types::Schema;
use crate::types::Traces;

#[async_trait::async_trait]
impl Dataset for Traces {
    fn datatype(&self) -> Datatype {
        Datatype::Traces
    }

    fn name(&self) -> &'static str {
        "blocks"
    }

    fn column_types(&self) -> HashMap<&'static str, ColumnType> {
        HashMap::from_iter(vec![
            ("action_from", ColumnType::Binary),
            ("action_to", ColumnType::Binary),
            ("action_value", ColumnType::Binary),
            ("action_gas", ColumnType::Binary),
            ("action_input", ColumnType::Binary),
            ("action_call_type", ColumnType::String),
            ("action_init", ColumnType::Binary),
            ("action_reward_type", ColumnType::String),
            ("action_type", ColumnType::String),
            ("result_gas_used", ColumnType::Binary),
            ("result_output", ColumnType::Binary),
            ("result_code", ColumnType::Binary),
            ("result_address", ColumnType::Binary),
            ("trace_address", ColumnType::String),
            ("subtraces", ColumnType::Int32),
            ("transaction_position", ColumnType::Int32),
            ("transaction_hash", ColumnType::Binary),
            ("block_number", ColumnType::Int64),
            ("block_hash", ColumnType::Binary),
            ("error", ColumnType::String),
        ])
    }

    fn default_columns(&self) -> Vec<&'static str> {
        vec![
            "action_from",
            "action_to",
            "action_value",
            "action_gas",
            "action_input",
            "action_call_type",
            "action_init",
            "action_reward_type",
            "action_type",
            "result_gas_used",
            "result_output",
            "result_code",
            "result_address",
            "trace_address",
            "subtraces",
            "transaction_position",
            "transaction_hash",
            "block_number",
            "block_hash",
            "error",
        ]
    }

    fn default_sort(&self) -> Vec<String> {
        vec!["block_number".to_string(), "trace_position".to_string()]
    }

    async fn collect_chunk(
        &self,
        block_chunk: &BlockChunk,
        opts: &FreezeOpts,
    ) -> Result<DataFrame, CollectError> {
        let numbers = block_chunk.numbers();
        let traces = fetch_traces(
            numbers,
            &opts.provider,
            &opts.max_concurrent_blocks,
            &opts.rate_limiter,
        )
        .await?;
        let df = traces_to_df(traces, &opts.schemas[&Datatype::Traces])
            .map_err(CollectError::PolarsError);
        if let Some(sort_keys) = opts.sort.get(&Datatype::Traces) {
            df.map(|x| x.sort(sort_keys, false))?
                .map_err(CollectError::PolarsError)
        } else {
            df
        }
    }
}

pub async fn fetch_traces(
    block_numbers: Vec<u64>,
    provider: &Provider<Http>,
    max_concurrent_blocks: &u64,
    rate_limiter: &Option<Arc<RateLimiter>>,
) -> Result<Vec<Trace>, CollectError> {
    let semaphore = Arc::new(Semaphore::new(*max_concurrent_blocks as usize));

    let futures = block_numbers.into_iter().map(|block_number| {
        let provider = provider.clone();
        let semaphore = Arc::clone(&semaphore);
        let rate_limiter = rate_limiter.as_ref().map(Arc::clone);
        tokio::spawn(async move {
            let _permit = Arc::clone(&semaphore).acquire_owned().await;
            if let Some(limiter) = rate_limiter {
                Arc::clone(&limiter).until_ready().await;
            }
            provider
                .trace_block(BlockNumber::Number(block_number.into()))
                .await
        })
    });

    let results: Vec<_> = join_all(futures)
        .await
        .into_iter()
        .map(|res| res.map_err(CollectError::TaskFailed))
        .collect();

    let mut traces: Vec<Trace> = Vec::new();
    for result in results {
        let block_traces = result?.map_err(CollectError::ProviderError)?;
        traces.extend(block_traces);
    }

    Ok(traces)
}

fn reward_type_to_string(reward_type: &RewardType) -> String {
    match reward_type {
        RewardType::Block => "reward".to_string(),
        RewardType::Uncle => "uncle".to_string(),
        RewardType::EmptyStep => "emtpy_step".to_string(),
        RewardType::External => "external".to_string(),
    }
}

fn action_type_to_string(action_type: &ActionType) -> String {
    match action_type {
        ActionType::Call => "call".to_string(),
        ActionType::Create => "create".to_string(),
        ActionType::Reward => "reward".to_string(),
        ActionType::Suicide => "suicide".to_string(),
    }
}

fn action_call_type_to_string(action_call_type: &CallType) -> String {
    match action_call_type {
        CallType::None => "none".to_string(),
        CallType::Call => "call".to_string(),
        CallType::CallCode => "call_code".to_string(),
        CallType::DelegateCall => "delegate_call".to_string(),
        CallType::StaticCall => "static_call".to_string(),
    }
}

pub fn traces_to_df(traces: Vec<Trace>, schema: &Schema) -> Result<DataFrame, PolarsError> {
    let include_action_from = schema.contains_key("action_from");
    let include_action_to = schema.contains_key("action_to");
    let include_action_value = schema.contains_key("action_value");
    let include_action_gas = schema.contains_key("action_gas");
    let include_action_input = schema.contains_key("action_input");
    let include_action_call_type = schema.contains_key("action_call_type");
    let include_action_init = schema.contains_key("action_init");
    let include_action_reward_type = schema.contains_key("action_reward_type");
    let include_action_type = schema.contains_key("action_type");
    let include_result_gas_used = schema.contains_key("result_gas_used");
    let include_result_output = schema.contains_key("result_output");
    let include_result_code = schema.contains_key("result_code");
    let include_result_address = schema.contains_key("result_address");
    let include_trace_address = schema.contains_key("trace_address");
    let include_subtraces = schema.contains_key("subtraces");
    let include_transaction_position = schema.contains_key("transaction_position");
    let include_transaction_hash = schema.contains_key("transaction_hash");
    let include_block_number = schema.contains_key("block_number");
    let include_block_hash = schema.contains_key("block_hash");
    let include_error = schema.contains_key("error");

    let mut action_from: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut action_to: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut action_value: Vec<String> = Vec::with_capacity(traces.len());
    let mut action_gas: Vec<Option<u64>> = Vec::with_capacity(traces.len());
    let mut action_input: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut action_call_type: Vec<Option<String>> = Vec::with_capacity(traces.len());
    let mut action_init: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut action_reward_type: Vec<Option<String>> = Vec::with_capacity(traces.len());
    let mut action_type: Vec<String> = Vec::with_capacity(traces.len());
    let mut result_gas_used: Vec<Option<u64>> = Vec::with_capacity(traces.len());
    let mut result_output: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut result_code: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut result_address: Vec<Option<Vec<u8>>> = Vec::with_capacity(traces.len());
    let mut trace_address: Vec<String> = Vec::with_capacity(traces.len());
    let mut subtraces: Vec<u32> = Vec::with_capacity(traces.len());
    let mut transaction_position: Vec<u32> = Vec::with_capacity(traces.len());
    let mut transaction_hash: Vec<Vec<u8>> = Vec::with_capacity(traces.len());
    let mut block_number: Vec<u64> = Vec::with_capacity(traces.len());
    let mut block_hash: Vec<Vec<u8>> = Vec::with_capacity(traces.len());
    let mut error: Vec<Option<String>> = Vec::with_capacity(traces.len());

    for trace in traces.iter() {
        if let (Some(tx_hash), Some(tx_pos)) = (trace.transaction_hash, trace.transaction_position)
        {
            // Call
            // from: from,
            // to: to,
            // value: value,
            // gas: gas,
            // input: input,
            // call_type: action_call_type, [None, Call, CallCode, DelegateCall, StaticCall]
            //
            // Create
            // from: from,
            // value: value,
            // gas: gas,
            // init: init,
            //
            // Suicide
            // address: from,
            // refund_address: to,
            // balance: value,
            //
            // Reward
            // author: to,
            // value: value,
            // reward_type: action_reward_type, [Block, Uncle, EmptyStep, External],

            match &trace.action {
                Action::Call(a) => {
                    if include_action_from {
                        action_from.push(Some(a.from.as_bytes().to_vec()));
                    }
                    if include_action_to {
                        action_to.push(Some(a.to.as_bytes().to_vec()));
                    }
                    if include_action_value {
                        action_value.push(a.value.to_string());
                    }
                    if include_action_gas {
                        action_gas.push(Some(a.gas.as_u64()));
                    }
                    if include_action_input {
                        action_input.push(Some(a.input.to_vec()));
                    }
                    if include_action_call_type {
                        action_call_type.push(Some(action_call_type_to_string(&a.call_type)));
                    }

                    if include_action_init {
                        action_init.push(None)
                    }
                    if include_action_reward_type {
                        action_reward_type.push(None)
                    }
                }
                Action::Create(action) => {
                    if include_action_from {
                        action_from.push(Some(action.from.as_bytes().to_vec()));
                    }
                    if include_action_value {
                        action_value.push(action.value.to_string());
                    }
                    if include_action_gas {
                        action_gas.push(Some(action.gas.as_u64()));
                    }
                    if include_action_init {
                        action_init.push(Some(action.init.to_vec()));
                    }

                    if include_action_to {
                        action_to.push(None)
                    }
                    if include_action_input {
                        action_input.push(None)
                    }
                    if include_action_call_type {
                        action_call_type.push(None)
                    }
                    if include_action_reward_type {
                        action_reward_type.push(None)
                    }
                }
                Action::Suicide(action) => {
                    if include_action_from {
                        action_from.push(Some(action.address.as_bytes().to_vec()));
                    }
                    if include_action_to {
                        action_to.push(Some(action.refund_address.as_bytes().to_vec()));
                    }
                    if include_action_value {
                        action_value.push(action.balance.to_string());
                    }

                    if include_action_gas {
                        action_gas.push(None)
                    }
                    if include_action_input {
                        action_input.push(None)
                    }
                    if include_action_call_type {
                        action_call_type.push(None)
                    }
                    if include_action_init {
                        action_init.push(None)
                    }
                    if include_action_reward_type {
                        action_reward_type.push(None)
                    }
                }
                Action::Reward(action) => {
                    if include_action_to {
                        action_to.push(Some(action.author.as_bytes().to_vec()));
                    }
                    if include_action_value {
                        action_value.push(action.value.to_string());
                    }
                    if include_action_reward_type {
                        action_reward_type.push(Some(reward_type_to_string(&action.reward_type)));
                    }

                    if include_action_from {
                        action_from.push(None)
                    }
                    if include_action_gas {
                        action_gas.push(None)
                    }
                    if include_action_input {
                        action_input.push(None)
                    }
                    if include_action_call_type {
                        action_call_type.push(None)
                    }
                    if include_action_init {
                        action_init.push(None)
                    }
                }
            }
            if include_action_type {
                action_type.push(action_type_to_string(&trace.action_type));
            }

            match &trace.result {
                Some(Res::Call(result)) => {
                    if include_result_gas_used {
                        result_gas_used.push(Some(result.gas_used.as_u64()));
                    }
                    if include_result_output {
                        result_output.push(Some(result.output.to_vec()));
                    }

                    if include_result_code {
                        result_code.push(None);
                    }
                    if include_result_address {
                        result_address.push(None);
                    }
                }
                Some(Res::Create(result)) => {
                    if include_result_gas_used {
                        result_gas_used.push(Some(result.gas_used.as_u64()));
                    }
                    if include_result_code {
                        result_code.push(Some(result.code.to_vec()));
                    }
                    if include_result_address {
                        result_address.push(Some(result.address.as_bytes().to_vec()));
                    }

                    if include_result_output {
                        result_output.push(None);
                    }
                }
                Some(Res::None) | None => {
                    if include_result_gas_used {
                        result_gas_used.push(None);
                    }
                    if include_result_output {
                        result_output.push(None);
                    }
                    if include_result_code {
                        result_code.push(None);
                    }
                    if include_result_address {
                        result_address.push(None);
                    }
                }
            }
            if include_trace_address {
                trace_address.push(
                    trace
                        .trace_address
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<String>>()
                        .join("_"),
                );
            }
            if include_subtraces {
                subtraces.push(trace.subtraces as u32);
            }
            if include_transaction_position {
                transaction_position.push(tx_pos as u32);
            }
            if include_transaction_hash {
                transaction_hash.push(tx_hash.as_bytes().to_vec());
            }
            if include_block_number {
                block_number.push(trace.block_number);
            }
            if include_block_hash {
                block_hash.push(trace.block_hash.as_bytes().to_vec());
            }
            if include_error {
                error.push(trace.error.clone());
            }
        }
    }

    let mut cols = Vec::new();
    if include_action_from {
        cols.push(Series::new("action_from", action_from));
    }
    if include_action_to {
        cols.push(Series::new("action_to", action_to));
    }
    if include_action_value {
        cols.push(Series::new("action_value", action_value));
    }
    if include_action_gas {
        cols.push(Series::new("action_gas", action_gas));
    }
    if include_action_input {
        cols.push(Series::new("action_input", action_input));
    }
    if include_action_call_type {
        cols.push(Series::new("action_call_type", action_call_type));
    }
    if include_action_init {
        cols.push(Series::new("action_init", action_init));
    }
    if include_action_reward_type {
        cols.push(Series::new("action_reward_type", action_reward_type));
    }
    if include_action_type {
        cols.push(Series::new("action_type", action_type));
    }
    if include_result_gas_used {
        cols.push(Series::new("result_gas_used", result_gas_used));
    }
    if include_result_output {
        cols.push(Series::new("result_output", result_output));
    }
    if include_result_code {
        cols.push(Series::new("result_code", result_code));
    }
    if include_result_address {
        cols.push(Series::new("result_address", result_address));
    }
    if include_trace_address {
        cols.push(Series::new("trace_address", trace_address));
    }
    if include_subtraces {
        cols.push(Series::new("subtraces", subtraces));
    }
    if include_transaction_position {
        cols.push(Series::new("transaction_position", transaction_position));
    }
    if include_transaction_hash {
        cols.push(Series::new("transaction_hash", transaction_hash));
    }
    if include_block_number {
        cols.push(Series::new("block_number", block_number));
    }
    if include_block_hash {
        cols.push(Series::new("block_hash", block_hash));
    }
    if include_error {
        cols.push(Series::new("error", error));
    }

    DataFrame::new(cols)
}