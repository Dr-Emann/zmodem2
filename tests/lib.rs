extern crate log;
extern crate zmodem;
#[macro_use]
extern crate lazy_static;
extern crate rand;

use std::fs;
use std::io::*;
use std::process::*;
use std::thread::spawn;
use tempfile::{tempdir, NamedTempFile};

struct InOut<R: Read, W: Write> {
    r: R,
    w: W,
}

impl<R: Read, W: Write> InOut<R, W> {
    pub fn new(r: R, w: W) -> InOut<R, W> {
        InOut { r, w }
    }
}

impl<R: Read, W: Write> Read for InOut<R, W> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.r.read(buf)
    }
}

impl<R: Read, W: Write> Write for InOut<R, W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.w.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.w.flush()
    }
}

lazy_static! {
    static ref RND_VALUES: Vec<u8> = {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let mut buf = vec![0; 1024 * 1024 * 11];
        rng.fill_bytes(&mut buf);
        buf
    };
}

#[test]
#[cfg(unix)]
fn recv_from_sz() {
    let mut f = NamedTempFile::with_prefix("recv_from_sz").unwrap();
    f.write_all(&RND_VALUES).unwrap();

    let mut sz = Command::new("sz")
        .arg(f.path())
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .expect("sz failed to run");

    let child_stdin = sz.stdin.as_mut().unwrap();
    let child_stdout = sz.stdout.as_mut().unwrap();
    let mut inout = InOut::new(child_stdout, child_stdin);

    let mut c = Cursor::new(Vec::new());
    zmodem::read(&mut inout, &mut (None, 0), &mut c).unwrap();

    let status = sz.wait().unwrap();
    assert!(status.success());

    assert_eq!(&*RND_VALUES, c.get_ref());
}

#[test]
#[cfg(unix)]
fn send_to_rz() {
    const FILE_NAME: &str = "send_to_rz";

    let dir = tempdir().unwrap();
    let expected_path = dir.path().join(FILE_NAME);

    let mut sz = Command::new("rz")
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .current_dir(dir.path())
        .spawn()
        .expect("rz failed to run");

    let child_stdin = sz.stdin.as_mut().unwrap();
    let child_stdout = sz.stdout.as_mut().unwrap();
    let mut inout = InOut::new(child_stdout, child_stdin);

    let len = RND_VALUES.len() as u32;
    let mut cur = Cursor::new(&*RND_VALUES);

    zmodem::write(&mut inout, &mut cur, FILE_NAME, Some(len)).unwrap();

    let status = sz.wait().unwrap();
    assert!(status.success());

    let received = fs::read(&expected_path).expect(&format!("open '{}'", expected_path.display()));
    assert_eq!(&*RND_VALUES, &received);
}

#[test]
fn lib_send_recv() {
    let (in_rx, in_tx) = os_pipe::pipe().unwrap();
    let (out_rx, out_tx) = os_pipe::pipe().unwrap();

    spawn(move || {
        let mut inout = InOut::new(out_rx, in_tx);

        let mut c = Cursor::new(&*RND_VALUES);

        zmodem::write(&mut inout, &mut c, "test", None).unwrap();
    });

    let mut c = Cursor::new(Vec::new());

    let mut inout = InOut::new(in_rx, out_tx);

    zmodem::read(&mut inout, &mut (None, 0), &mut c).unwrap();

    assert_eq!(&*RND_VALUES, c.get_ref());
}
