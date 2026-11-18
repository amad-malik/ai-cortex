//! A tiny ustar writer for the tests that need a real release archive (the same format group_unpack builds by hand; the
//! hostile-archive tests there keep their own copy so that they do not depend on a shared helper).
#![allow(dead_code)]

use std::io::Write;

fn octal(buf: &mut [u8], v: u64) {
    let s = format!("{:0width$o}\0", v, width = buf.len() - 1);
    buf.copy_from_slice(s.as_bytes());
}

fn header(name: &[u8], ty: u8, size: u64, mode: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name);
    octal(&mut h[100..108], u64::from(mode));
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], 0);
    h[156] = ty;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].fill(b' ');
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

fn entry(h: [u8; 512], data: &[u8]) -> Vec<u8> {
    let mut v = h.to_vec();
    v.extend_from_slice(data);
    v.resize(v.len().div_ceil(512) * 512, 0);
    v
}

pub fn file(name: &str, data: &[u8], mode: u32) -> Vec<u8> {
    entry(header(name.as_bytes(), b'0', data.len() as u64, mode), data)
}

pub fn dir(name: &str) -> Vec<u8> {
    entry(header(name.as_bytes(), b'5', 0, 0o755), b"")
}

/// The entries and the two zero blocks that end an archive.
pub fn finish(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut v: Vec<u8> = entries.concat();
    v.extend_from_slice(&[0u8; 1024]);
    v
}

pub fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}
