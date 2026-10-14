// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Little-endian field access for on-disk structures.

pub(crate) fn get16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
pub(crate) fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
pub(crate) fn get64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
pub(crate) fn put(b: &mut [u8], at: usize, v: &[u8]) {
    b[at..at + v.len()].copy_from_slice(v);
}
pub(crate) fn put16(b: &mut [u8], at: usize, v: u16) {
    put(b, at, &v.to_le_bytes());
}
pub(crate) fn put32(b: &mut [u8], at: usize, v: u32) {
    put(b, at, &v.to_le_bytes());
}
pub(crate) fn put64(b: &mut [u8], at: usize, v: u64) {
    put(b, at, &v.to_le_bytes());
}
