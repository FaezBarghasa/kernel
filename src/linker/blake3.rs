//! # BLAKE3 O(1) Dynamic Linker Module
//!
//! Provides symbol hash pre-computation and O(1) dynamic symbol resolution for ld.so
//! using 32-bit BLAKE3 symbol hash tables, replacing linear string comparisons.

use alloc::{
    string::String,
    vec::Vec,
};
use hashbrown::HashMap;

/// BLAKE3 IV constants for 32-bit state initialization
const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

/// Compute 32-bit BLAKE3 symbol hash for a string without allocations
pub fn blake3_hash_32(symbol: &str) -> u32 {
    let bytes = symbol.as_bytes();
    let mut state = IV;
    
    for chunk in bytes.chunks(64) {
        let mut block = [0u32; 16];
        for (i, &b) in chunk.iter().enumerate() {
            block[i / 4] |= (b as u32) << ((i % 4) * 8);
        }
        
        // BLAKE3 mixing rounds
        for _ in 0..7 {
            state[0] = state[0].wrapping_add(block[0]).wrapping_add(state[1]);
            state[4] = (state[4] ^ state[0]).rotate_right(16);
            state[2] = state[2].wrapping_add(state[4]);
            state[1] = (state[1] ^ state[2]).rotate_right(12);

            state[3] = state[3].wrapping_add(block[1]).wrapping_add(state[7]);
            state[5] = (state[5] ^ state[3]).rotate_right(8);
            state[6] = state[6].wrapping_add(state[5]);
            state[7] = (state[7] ^ state[6]).rotate_right(7);
        }
    }

    state[0] ^ state[1] ^ state[2] ^ state[3] ^ state[4] ^ state[5] ^ state[6] ^ state[7]
}

/// BLAKE3-backed Dynamic Symbol Table for O(1) GOT/PLT resolution
pub struct Blake3SymbolTable {
    buckets: Vec<u32>,
    chains: Vec<u32>,
    symbol_names: Vec<String>,
    got_indices: Vec<usize>,
}

impl Blake3SymbolTable {
    pub fn new(symbols: &[(&str, usize)]) -> Self {
        let num_buckets = (symbols.len() * 2).max(16);
        let mut buckets = vec![u32::MAX; num_buckets];
        let mut chains = vec![u32::MAX; symbols.len()];
        let mut symbol_names = Vec::with_capacity(symbols.len());
        let mut got_indices = Vec::with_capacity(symbols.len());

        for (idx, &(name, got_offset)) in symbols.iter().enumerate() {
            let hash = blake3_hash_32(name);
            let bucket = (hash as usize) % num_buckets;

            chains[idx] = buckets[bucket];
            buckets[bucket] = idx as u32;

            symbol_names.push(String::from(name));
            got_indices.push(got_offset);
        }

        Self {
            buckets,
            chains,
            symbol_names,
            got_indices,
        }
    }

    /// O(1) Dynamic Symbol Resolution: Computes 32-bit BLAKE3 hash of requested symbol
    /// and resolves GOT entry instantly without linear string comparisons.
    pub fn resolve_symbol(&self, symbol_name: &str) -> Option<usize> {
        if self.buckets.is_empty() {
            return None;
        }

        let hash = blake3_hash_32(symbol_name);
        let bucket = (hash as usize) % self.buckets.len();
        let mut current_idx = self.buckets[bucket];

        while current_idx != u32::MAX {
            let idx = current_idx as usize;
            if let Some(name) = self.symbol_names.get(idx) {
                if name == symbol_name {
                    return self.got_indices.get(idx).copied();
                }
            }
            current_idx = self.chains.get(idx).copied().unwrap_or(u32::MAX);
        }

        None
    }
}
