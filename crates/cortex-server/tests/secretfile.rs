//! A secret read from a file (the database passwords). DB-free. RED first.
//! Rules: a REGULAR file, never through a symlink (O_NOFOLLOW) and never blocking on a FIFO, at most 4096 bytes, UTF-8 without NUL,
//! only ONE final line ending is not part of the secret, empty is an error; the error carries neither path nor content.

use cortex_server::secretfile::{MAX_SECRET_BYTES, read_secret_file, read_secret_from};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("secretfile-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn put(d: &Path, bytes: &[u8]) -> PathBuf {
    let p = d.join("pw");
    fs::write(&p, bytes).expect("seed");
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).expect("mode");
    p
}

#[test]
fn sf_01_a_plain_secret_is_read_and_one_final_line_ending_is_dropped() {
    let d = dir("plain");
    assert_eq!(
        read_secret_file(&put(&d, b"s3cret"))
            .expect("read")
            .expose(),
        "s3cret"
    );
    assert_eq!(
        read_secret_file(&put(&d, b"s3cret\n"))
            .expect("read")
            .expose(),
        "s3cret"
    );
    assert_eq!(
        read_secret_file(&put(&d, b"s3cret\r\n"))
            .expect("read")
            .expose(),
        "s3cret"
    );
    assert_eq!(
        read_secret_file(&put(&d, b"s3cret\n\n"))
            .expect("read")
            .expose(),
        "s3cret\n",
        "only ONE line ending is not part of the secret"
    );
    assert_eq!(
        read_secret_file(&put(&d, b" a b ")).expect("read").expose(),
        " a b ",
        "no other trimming: a password may hold spaces"
    );
    assert_eq!(
        read_secret_file(&put(&d, "pässwörd".as_bytes()))
            .expect("read")
            .expose(),
        "pässwörd"
    );
}

#[test]
fn sf_02_empty_nul_non_utf8_and_oversize_are_refused() {
    let d = dir("bad");
    for (why, bytes) in [
        ("empty", b"".to_vec()),
        ("only a newline", b"\n".to_vec()),
        ("only CRLF", b"\r\n".to_vec()),
        ("a NUL", b"ab\0cd".to_vec()),
        ("not UTF-8", vec![0xff, 0xfe, b'a']),
        ("over the cap", vec![b'a'; MAX_SECRET_BYTES as usize + 1]),
    ] {
        assert!(read_secret_file(&put(&d, &bytes)).is_err(), "{why}");
    }
    assert_eq!(MAX_SECRET_BYTES, 4096);
    assert!(
        read_secret_file(&put(&d, &vec![b'a'; MAX_SECRET_BYTES as usize])).is_ok(),
        "exactly the cap is fine"
    );
}

#[test]
fn sf_03_a_symlink_a_directory_a_fifo_and_a_missing_file_are_refused() {
    let d = dir("odd");
    let real = put(&d, b"s3cret");
    let link = d.join("link");
    symlink(&real, &link).expect("link");
    assert!(
        read_secret_file(&link).is_err(),
        "a symlink is never followed"
    );
    symlink(d.join("nowhere"), d.join("dangling")).expect("link");
    assert!(read_secret_file(&d.join("dangling")).is_err());
    assert!(read_secret_file(&d).is_err(), "a directory");
    assert!(read_secret_file(&d.join("missing")).is_err());
    let fifo = d.join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo")
            .success()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let p = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_secret_file(&p).is_err());
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("blocked on a FIFO"),
        "a FIFO is refused without blocking"
    );
}

#[test]
fn sf_04_the_error_names_neither_the_path_nor_the_content() {
    let d = dir("leak");
    let p = d.join("pw-secret-path-ZZ9");
    fs::write(&p, vec![b'x'; 5000]).expect("seed");
    let e = read_secret_file(&p).err().expect("over the cap");
    let all = format!("{e} {e:?}");
    assert!(
        !all.contains("pw-secret-path-ZZ9") && !all.contains("xxxx"),
        "{all}"
    );
    let e2 = read_secret_file(&d.join("pw-secret-path-ZZ9-missing"))
        .err()
        .expect("missing");
    assert!(!format!("{e2} {e2:?}").contains("ZZ9"));
}

/// vera SERVER-I4-V1: the reader must hand ONLY slices of its one zeroised buffer to the underlying reader. Rust's `read_to_end` reads
/// into a 32-byte STACK probe when fewer than 32 bytes of spare capacity remain, then copies: a second, un-zeroised copy of the tail.
/// This reader records where every read lands; a legal 4066-byte first read plus a 30-byte second read (4096 in all) reaches that path.
struct Script {
    chunks: std::collections::VecDeque<Vec<u8>>,
    calls: Vec<(usize, usize)>,
    fail_once: Option<std::io::ErrorKind>,
}

impl Script {
    fn new(chunks: Vec<Vec<u8>>) -> Self {
        Script {
            chunks: chunks.into(),
            calls: Vec::new(),
            fail_once: None,
        }
    }
}

impl std::io::Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.calls.push((buf.as_ptr() as usize, buf.len()));
        if let Some(kind) = self.fail_once.take() {
            return Err(std::io::Error::from(kind));
        }
        let Some(front) = self.chunks.front_mut() else {
            return Ok(0);
        };
        let n = front.len().min(buf.len());
        buf[..n].copy_from_slice(&front[..n]);
        front.drain(..n);
        if front.is_empty() {
            self.chunks.pop_front();
        }
        Ok(n)
    }
}

#[test]
fn ss_01_every_read_lands_inside_the_one_zeroised_buffer() {
    let mut s = Script::new(vec![vec![b'a'; 4066], vec![b'b'; 30]]);
    let secret = read_secret_from(&mut s).expect("4096 bytes are legal");
    assert_eq!(secret.expose().len(), 4096);
    let (base, first_len) = s.calls[0];
    assert!(
        first_len > MAX_SECRET_BYTES as usize,
        "the first read is handed the WHOLE buffer, cap plus one: {first_len}"
    );
    for (i, (ptr, len)) in s.calls.iter().enumerate() {
        assert!(
            *ptr >= base && ptr + len <= base + first_len,
            "read {i} landed outside the buffer: offset {} len {len}",
            (*ptr).wrapping_sub(base)
        );
    }
}

#[test]
fn ss_02_cap_plus_one_is_refused_and_an_interrupted_read_is_retried() {
    let mut s = Script::new(vec![vec![b'a'; 4097]]);
    assert!(
        read_secret_from(&mut s).is_err(),
        "4097 bytes are one too many"
    );
    let mut s = Script::new(vec![b"s3cret\n".to_vec()]);
    s.fail_once = Some(std::io::ErrorKind::Interrupted);
    assert_eq!(read_secret_from(&mut s).expect("retry").expose(), "s3cret");
}

#[test]
fn ss_03_a_read_error_is_the_one_fixed_error() {
    let mut s = Script::new(vec![b"s3cret".to_vec()]);
    s.fail_once = Some(std::io::ErrorKind::PermissionDenied);
    let e = read_secret_from(&mut s).err().expect("must fail");
    assert_eq!(e.to_string(), "the secret file could not be read");
}

/// hale F-2..F-4 (packet 4): her probes, adopted. sp_a: a FIFO that holds a password is never read (the regular-file rule); sp_b: a character
/// split across two reads is read whole; sp_c: the cap is on the bytes (4096 plus a line ending is over, 4096 one-byte reads are fine);
/// sp_d: a byte order mark is part of the secret (stated behaviour, not dropped silently).
#[test]
fn sp_a_a_fifo_that_holds_a_password_and_has_no_writer_left_is_never_read() {
    // Deterministic, as hl_18 is for the health marker: keep a reader open so the pipe keeps its bytes, push the password in with a writer,
    // let the writer go; the next reader would get the bytes and then EOF.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let d = dir("fifopw");
    let fifo = d.join("pw");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo")
            .success()
    );
    let _keep = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo)
        .expect("keep a reader");
    let mut w = fs::OpenOptions::new()
        .write(true)
        .open(&fifo)
        .expect("writer");
    w.write_all(b"s3cret\n").expect("fill the pipe");
    drop(w);
    let (tx, rx) = std::sync::mpsc::channel();
    let p = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_secret_file(&p).map(|s| s.expose().to_owned()));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("blocked on a FIFO");
    assert!(got.is_err(), "a password was read from a FIFO");
}

#[test]
fn sp_b_one_multibyte_character_split_across_two_reads_is_read_whole() {
    let mut s = Script::new(vec![b"p\xc3".to_vec(), b"\xa4ss\n".to_vec()]);
    let secret = read_secret_from(&mut s).expect("valid UTF-8 once whole");
    assert_eq!(secret.expose(), "päss");
}

#[test]
fn sp_c_4096_one_byte_reads_are_fine_and_a_secret_of_4096_plus_a_newline_is_over_the_cap() {
    let ok = Script::new((0..4096).map(|_| vec![b'a']).collect());
    let mut ok = ok;
    assert_eq!(
        read_secret_from(&mut ok)
            .expect("4096 bytes")
            .expose()
            .len(),
        4096
    );
    let mut over = Script::new(vec![vec![b'a'; 4096], b"\n".to_vec()]);
    let r = read_secret_from(&mut over);
    assert!(r.is_err(), "the cap is on the file's bytes");
}

#[test]
fn sp_d_a_byte_order_mark_is_part_of_the_secret_not_silently_dropped() {
    let d = dir("bom");
    let got = read_secret_file(&put(&d, b"\xef\xbb\xbfpw\n")).expect("read");
    assert!(got.expose().starts_with('\u{feff}'));
}
