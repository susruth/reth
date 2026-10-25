//! How long block execution waits on signature recovery for its next transaction.
//!
//! The consumer mirrors the execution loop: it takes transactions in block order the way
//! `PayloadHandle::iter_transactions` does, times each wait (reth's `transaction_wait`), and then
//! spins for a fixed per-transaction execution time.
#![allow(missing_docs)]

use alloy_consensus::transaction::SignerRecoverable;
use criterion::{criterion_group, criterion_main, Criterion};
use reth_chainspec::ChainSpec;
use reth_engine_tree::tree::{payload_processor::PayloadProcessor, TreeConfig};
use reth_ethereum_primitives::TransactionSigned;
use reth_evm::precompile_cache::PrecompileCacheMap;
use reth_evm_ethereum::EthEvmConfig;
use reth_tasks::{RuntimeBuilder, RuntimeConfig};
use reth_testing_utils::generators::{self, random_signed_tx};
use std::{
    collections::VecDeque,
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

/// Simulated execution time per transaction, close to an ERC-20 transfer on a laptop.
const EXECUTION: Duration = Duration::from_micros(8);

fn spin(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        std::hint::spin_loop();
    }
}

/// Converts and consumes one block, returning (total wait, wait for the first transaction, wall
/// time).
fn run_block(
    processor: &PayloadProcessor<EthEvmConfig>,
    txs: Vec<TransactionSigned>,
) -> (Duration, Duration, Duration) {
    let count = txs.len();
    let start = Instant::now();
    let (_, receiver) = processor.spawn_tx_iterator(
        (txs, |tx: TransactionSigned| tx.try_into_recovered()),
        count,
        false,
        false,
    );
    let mut pending = VecDeque::new();
    let (mut wait, mut first) = (Duration::ZERO, Duration::ZERO);
    for next in 0..count {
        let wait_start = Instant::now();
        while !matches!(pending.front(), Some(Some(_))) {
            let (idx, tx) = receiver.recv().unwrap();
            let offset = idx - next;
            if offset >= pending.len() {
                pending.resize_with(offset + 1, || None);
            }
            pending[offset] = Some(tx);
        }
        let tx = pending.pop_front().flatten().unwrap();
        wait += wait_start.elapsed();
        if next == 0 {
            first = wait;
        }
        black_box(tx.unwrap());
        spin(EXECUTION);
    }
    (wait, first, start.elapsed())
}

fn tx_iterator(c: &mut Criterion) {
    let runtime = RuntimeBuilder::new(RuntimeConfig::default()).build().unwrap();
    let processor = PayloadProcessor::new(
        runtime,
        EthEvmConfig::new(Arc::new(ChainSpec::default())),
        &TreeConfig::default(),
        PrecompileCacheMap::default(),
    );
    let mut rng = generators::rng();
    let mut group = c.benchmark_group("tx_iterator");
    for count in [200, 1000, 4000] {
        let txs = (0..count).map(|_| random_signed_tx(&mut rng)).collect::<Vec<_>>();
        group.bench_function(format!("wait/{count}"), |b| {
            b.iter_custom(|iters| (0..iters).map(|_| run_block(&processor, txs.clone()).0).sum())
        });
        group.bench_function(format!("first/{count}"), |b| {
            b.iter_custom(|iters| (0..iters).map(|_| run_block(&processor, txs.clone()).1).sum())
        });
        group.bench_function(format!("block/{count}"), |b| {
            b.iter_custom(|iters| (0..iters).map(|_| run_block(&processor, txs.clone()).2).sum())
        });
    }
    group.finish();
}

criterion_group!(benches, tx_iterator);
criterion_main!(benches);
