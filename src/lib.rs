// SPDX-License-Identifier: MIT OR Apache-2.0
//! ZMODEM file transfer protocol

use binread::{io::Cursor, BinRead, BinReaderExt, NullString};
use bitflags::bitflags;
use core::convert::TryFrom;
use crc::{Crc, CRC_16_XMODEM, CRC_32_ISO_HDLC};
use std::fmt::{self, Display};
use std::io::{Read, Seek, SeekFrom, Write};
use tinyvec::{array_vec, ArrayVec};

const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_XMODEM);
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

const ZPAD: u8 = b'*';
const ZDLE: u8 = 0x18;
const XON: u8 = 0x11;

const ZACK_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZACK);
const ZDATA_HEADER: Header = Header::new(Encoding::ZBIN32, Frame::ZDATA);
const ZEOF_HEADER: Header = Header::new(Encoding::ZBIN32, Frame::ZEOF);
const ZFIN_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZFIN);
const ZNAK_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZNAK);
const ZRPOS_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZRPOS);
const ZRQINIT_HEADER: Header = Header::new(Encoding::ZHEX, Frame::ZRQINIT);

const SUBPACKET_SIZE: u32 = 1024;
const SUBPACKET_PER_ACK: usize = 10;
/// Buffer size with enough capacity for an escaped header.
const HEADER_SIZE: usize = 32;

/// Receive buffer
type RxBuffer = ArrayVec<[u8; 2048]>;
/// Transmit buffer. The size is picked based on maximum subpacket size in the
/// original 1988 ZMODEM specification.
type TxBuffer = ArrayVec<[u8; 1024]>;

/// https://play.rust-lang.org/?version=stable&mode=debug&edition=2021&gist=20db24d9f0aaff4d13f0144416f34d46
const ZDLE_TABLE: [u8; 0x100] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x4d, 0x0e, 0x0f,
    0x50, 0x51, 0x12, 0x53, 0x14, 0x15, 0x16, 0x17, 0x58, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
    0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f,
    0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f,
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f,
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x6c,
    0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0xcd, 0x8e, 0x8f,
    0xd0, 0xd1, 0x92, 0xd3, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f,
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
    0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf,
    0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcc, 0xcd, 0xce, 0xcf,
    0xd0, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xdb, 0xdc, 0xdd, 0xde, 0xdf,
    0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef,
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0x6d,
];

/// https://play.rust-lang.org/?version=stable&mode=debug&edition=2021&gist=20db24d9f0aaff4d13f0144416f34d46
pub const UNZDLE_TABLE: [u8; 0x100] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x7f, 0xff, 0x6e, 0x6f,
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f,
    0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f,
    0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f,
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
    0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xbb, 0xbc, 0xbd, 0xbe, 0xbf,
    0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f,
    0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f,
    0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef,
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InvalidData;

pub trait Reader {
    fn read(&mut self, buf: &mut [u8]) -> Result<u32, InvalidData>;
    fn seek(&mut self, offset: u32) -> Result<u32, InvalidData>;
}

impl<R> Reader for R
where
    R: Read + Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<u32, InvalidData> {
        Ok(self.read(buf).or(Err(InvalidData))? as u32)
    }

    fn seek(&mut self, offset: u32) -> Result<u32, InvalidData> {
        Ok(self
            .seek(SeekFrom::Start(offset as u64))
            .or(Err(InvalidData))? as u32)
    }
}

pub trait Writer {
    fn write(&mut self, buf: &[u8]) -> Result<(), InvalidData>;
}

impl<W> Writer for W
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> Result<(), InvalidData> {
        self.write_all(buf).or(Err(InvalidData))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    encoding: Encoding,
    kind: Frame,
    flags: [u8; 4],
}

impl Header {
    pub const fn new(encoding: Encoding, kind: Frame) -> Self {
        Self {
            encoding,
            kind,
            flags: [0; 4],
        }
    }

    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub const fn kind(&self) -> Frame {
        self.kind
    }

    pub const fn count(&self) -> u32 {
        u32::from_le_bytes(self.flags)
    }

    pub fn write_zrinit<P>(
        port: &mut P,
        encoding: Encoding,
        zrinit: Zrinit,
        count: u16,
    ) -> core::result::Result<(), InvalidData>
    where
        P: Write,
    {
        let count = count.to_le_bytes();
        Self {
            encoding,
            kind: Frame::ZRINIT,
            flags: [count[0], count[1], 0, zrinit.bits()],
        }
        .write(port)
        .or(Err(InvalidData))
    }

    pub fn write_zfile<P>(
        port: &mut P,
        name: &str,
        size: u32,
    ) -> core::result::Result<(), InvalidData>
    where
        P: Write,
    {
        let mut tx_buf = TxBuffer::new();

        tx_buf.truncate(0);
        tx_buf.extend_from_slice(name.as_bytes());
        tx_buf.push(b'\0');
        // FIXME: Uses heap:
        tx_buf.extend_from_slice(size.to_string().as_bytes());
        tx_buf.push(b'\0');

        Self {
            encoding: Encoding::ZBIN32,
            kind: Frame::ZFILE,
            flags: [0; 4],
        }
        .write(port)?;

        write_subpacket(port, Encoding::ZBIN32, Packet::ZCRCW, &tx_buf)
    }

    pub fn read_zfile<P>(&self, port: &mut P) -> core::result::Result<Option<File>, InvalidData>
    where
        P: Read + Write,
    {
        let mut rx_buf = RxBuffer::new();
        let result = read_subpacket(port, self.encoding(), &mut rx_buf);
        match result {
            Ok(_) => {
                ZRPOS_HEADER.with_count(0).write(port)?;
                let reader: ZfileReader = Cursor::new(rx_buf).read_ne().or(Err(InvalidData))?;
                if reader.file_name.len() > 255 {
                    return Err(InvalidData);
                }
                let mut name = [0; 256];
                for (i, b) in reader.file_name.as_slice().iter().enumerate() {
                    name[i] = *b;
                }
                Ok(Some(File { name }))
            }
            _ => ZNAK_HEADER.write(port).and(Ok(None)),
        }
    }

    pub fn write<P>(&self, port: &mut P) -> core::result::Result<(), InvalidData>
    where
        P: Write,
    {
        let mut out = array_vec!([u8; HEADER_SIZE]);
        out.push(ZPAD);
        if self.encoding == Encoding::ZHEX {
            out.push(ZPAD);
        }
        out.push(ZDLE);
        out.push(self.encoding as u8);
        out.push(self.kind as u8);
        out.extend_from_slice(&self.flags);
        // Skips ZPAD and encoding:
        let data = if self.encoding == Encoding::ZHEX {
            &out[4..]
        } else {
            &out[3..]
        };
        let mut crc = [0u8; 4];
        let crc_len = make_crc(data, &mut crc, self.encoding);
        out.extend_from_slice(&crc[..crc_len]);
        // Skips ZPAD and encoding:
        if self.encoding == Encoding::ZHEX {
            let hex = hex::encode(&out[4..]);
            out.truncate(4);
            out.extend_from_slice(hex.as_bytes());
        }
        let mut escaped = [0u8; HEADER_SIZE];
        // Does not corrupt `ZHEX` as the encoding byte is not escaped:
        let escaped_len = escape_mem(&out[3..], &mut escaped[0..HEADER_SIZE]);
        out.truncate(3);
        out.extend_from_slice(&escaped[..escaped_len]);
        if self.encoding == Encoding::ZHEX {
            // Add trailing CRLF for ZHEX transfer:
            out.extend_from_slice(b"\r\n");
            if self.kind != Frame::ZACK && self.kind != Frame::ZFIN {
                out.push(XON);
            }
        }
        port.write_all(&out).or(Err(InvalidData))
    }

    pub fn read<P>(port: &mut P) -> core::result::Result<Header, InvalidData>
    where
        P: Read,
    {
        let encoding = Encoding::try_from(read_byte(port)?)?;
        let mut out = array_vec!([u8; HEADER_SIZE]);
        for _ in 0..Header::unescaped_size(encoding) - 1 {
            out.push(read_byte_unescaped(port)?);
        }
        if encoding == Encoding::ZHEX {
            hex::decode_in_slice(&mut out).or(Err(InvalidData))?;
            out.truncate(out.len() / 2);
        }
        check_crc(&out[..5], &out[5..], encoding)?;
        let kind = Frame::try_from(out[0])?;
        let mut header = Header::new(encoding, kind);
        header.flags.copy_from_slice(&out[1..=4]);
        Ok(header)
    }

    pub const fn with_count(&self, count: u32) -> Self {
        Header {
            encoding: self.encoding,
            kind: self.kind,
            flags: count.to_le_bytes(),
        }
    }

    pub const fn with_flags(&self, flags: &[u8; 4]) -> Self {
        Header {
            encoding: self.encoding,
            kind: self.kind,
            flags: *flags,
        }
    }

    const fn unescaped_size(encoding: Encoding) -> usize {
        match encoding {
            Encoding::ZBIN => core::mem::size_of::<Header>() + 2,
            Encoding::ZBIN32 => core::mem::size_of::<Header>() + 4,
            // Encoding is stored as a single byte also for ZHEX, thus the
            // subtraction:
            Encoding::ZHEX => (core::mem::size_of::<Header>() + 2) * 2 - 1,
        }
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:8} {}", self.encoding, self.kind)
    }
}

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq)]
/// Frame encodings
pub enum Encoding {
    ZBIN = 0x41,
    ZHEX = 0x42,
    ZBIN32 = 0x43,
}

const ENCODINGS: &[Encoding] = &[Encoding::ZBIN, Encoding::ZHEX, Encoding::ZBIN32];

impl TryFrom<u8> for Encoding {
    type Error = InvalidData;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        ENCODINGS
            .iter()
            .find(|e| value == **e as u8)
            .map_or(Err(InvalidData), |e| Ok(*e))
    }
}

impl Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#02x}", *self as u8)
    }
}

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq)]
/// Frame types
pub enum Frame {
    /// Request receive init
    ZRQINIT = 0,
    /// Receiver capabilities and packet size
    ZRINIT = 1,
    /// Send init sequence (optional)
    ZSINIT = 2,
    /// ACK to above
    ZACK = 3,
    /// File name from sender
    ZFILE = 4,
    /// To sender: skip this file
    ZSKIP = 5,
    /// Last packet was garbled
    ZNAK = 6,
    /// Abort batch transfers
    ZABORT = 7,
    /// Finish session
    ZFIN = 8,
    /// Resume data trans at this position
    ZRPOS = 9,
    /// Data packet(s) follow
    ZDATA = 10,
    /// End of file
    ZEOF = 11,
    /// Fatal Read or Write error Detected
    ZFERR = 12,
    /// Request for file CRC and response
    ZCRC = 13,
    /// Receiver's Challenge
    ZCHALLENGE = 14,
    /// Request is complete
    ZCOMPL = 15,
    /// Other end canned session with CAN*5
    ZCAN = 16,
    /// Request for free bytes on filesystem
    ZFREECNT = 17,
    /// Command from sending program
    ZCOMMAND = 18,
    ///  Output to standard error, data follows
    ZSTDERR = 19,
}

const FRAMES: &[Frame] = &[
    Frame::ZRQINIT,
    Frame::ZRINIT,
    Frame::ZSINIT,
    Frame::ZACK,
    Frame::ZFILE,
    Frame::ZSKIP,
    Frame::ZNAK,
    Frame::ZABORT,
    Frame::ZFIN,
    Frame::ZRPOS,
    Frame::ZDATA,
    Frame::ZEOF,
    Frame::ZFERR,
    Frame::ZCRC,
    Frame::ZCHALLENGE,
    Frame::ZCOMPL,
    Frame::ZCAN,
    Frame::ZFREECNT,
    Frame::ZCOMMAND,
    Frame::ZSTDERR,
];

impl TryFrom<u8> for Frame {
    type Error = InvalidData;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        FRAMES
            .iter()
            .find(|t| value == **t as u8)
            .map_or(Err(InvalidData), |t| Ok(*t))
    }
}

impl Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#02x}", *self as u8)
    }
}

bitflags! {
   /// `ZRINIT` flags
   pub struct Zrinit: u8 {
        /// Can send and receive in full-duplex
        const CANFDX = 0x01;
        /// Can receive data in parallel with disk I/O
        const CANOVIO = 0x02;
        /// Can send a break signal
        const CANBRK = 0x04;
        /// Can decrypt
        const CANCRY = 0x08;
        /// Can uncompress
        const CANLZW = 0x10;
        /// Can use 32-bit frame check
        const CANFC32 = 0x20;
        /// Expects control character to be escaped
        const ESCCTL = 0x40;
        /// Expects 8th bit to be escaped
        const ESC8 = 0x80;
    }
}

#[derive(PartialEq)]
pub struct File {
    name: [u8; 256],
}

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq)]
/// The ZMODEM subpacket type
pub enum Packet {
    ZCRCE = 0x68,
    ZCRCG = 0x69,
    ZCRCQ = 0x6a,
    ZCRCW = 0x6b,
}

const PACKETS: &[Packet] = &[Packet::ZCRCE, Packet::ZCRCG, Packet::ZCRCQ, Packet::ZCRCW];

impl TryFrom<u8> for Packet {
    type Error = InvalidData;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        PACKETS
            .iter()
            .find(|e| value == **e as u8)
            .map_or(Err(InvalidData), |e| Ok(*e))
    }
}

impl Display for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#02x}", *self as u8)
    }
}

#[derive(BinRead)]
#[br(assert(file_name.len() != 0))]
struct ZfileReader {
    file_name: NullString,
}

#[derive(PartialEq)]
enum Stage {
    Waiting,
    Ready,
    Receiving,
}

/// Sends a file using the ZMODEM file transfer protocol.
pub fn write<P, F>(
    port: &mut P,
    file: &mut F,
    name: &str,
    size: Option<u32>,
) -> core::result::Result<(), InvalidData>
where
    P: Read + Write,
    F: Reader,
{
    let mut stage = Stage::Waiting;

    ZRQINIT_HEADER.write(port)?;
    loop {
        if read_zpad(port).is_err() {
            continue;
        }
        let frame = match Header::read(port) {
            Err(_) => {
                ZNAK_HEADER.write(port)?;
                continue;
            }
            Ok(frame) => frame,
        };
        match frame.kind() {
            Frame::ZRINIT => match stage {
                Stage::Waiting => {
                    let size = size.unwrap_or(0);
                    Header::write_zfile(port, name, size)?;
                    stage = Stage::Ready;
                }
                Stage::Ready => (),
                Stage::Receiving => ZFIN_HEADER.write(port)?,
            },
            Frame::ZRPOS | Frame::ZACK => {
                if stage == Stage::Waiting {
                    ZRQINIT_HEADER.write(port)?;
                } else {
                    write_zdata(port, file, &frame)?;
                    stage = Stage::Receiving;
                }
            }
            _ => {
                if stage == Stage::Waiting {
                    ZRQINIT_HEADER.write(port)?;
                } else {
                    port.write_all("OO".as_bytes()).or(Err(InvalidData))?;
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Receives a file using the ZMODEM file transfer protocol.
pub fn read<P, F>(
    port: &mut P,
    state: &mut (Option<File>, u32),
    out: &mut F,
) -> core::result::Result<(), InvalidData>
where
    P: Read + Write,
    F: Writer,
{
    if state.0.is_none() {
        assert_eq!(state.1, 0);
        Header::write_zrinit(
            port,
            Encoding::ZHEX,
            Zrinit::CANCRY | Zrinit::CANOVIO | Zrinit::CANFC32,
            0,
        )?
    }

    loop {
        if read_zpad(port).is_err() {
            continue;
        }
        let frame = match Header::read(port) {
            Err(_) => {
                ZNAK_HEADER.write(port)?;
                continue;
            }
            Ok(frame) => frame,
        };
        match frame.kind() {
            Frame::ZFILE => {
                if state.0.is_none() || state.1 == 0 {
                    assert_eq!(state.1, 0);
                    state.0 = frame.read_zfile(port)?;
                }
            }
            Frame::ZDATA => {
                if state.0.is_none() {
                    Header::write_zrinit(
                        port,
                        Encoding::ZHEX,
                        Zrinit::CANCRY | Zrinit::CANOVIO | Zrinit::CANFC32,
                        0,
                    )?;
                } else if frame.count() != state.1 {
                    ZRPOS_HEADER.with_count(state.1).write(port)?
                } else {
                    read_zdata(frame.encoding() as u8, &mut state.1, port, out)?;
                }
            }
            Frame::ZEOF if state.0.is_some() => {
                if frame.count() != state.1 {
                    log::error!(
                        "ZEOF offset mismatch: frame({}) != recv({})",
                        frame.count(),
                        state.1
                    );
                } else {
                    Header::write_zrinit(
                        port,
                        Encoding::ZHEX,
                        Zrinit::CANCRY | Zrinit::CANOVIO | Zrinit::CANFC32,
                        0,
                    )?
                }
            }
            Frame::ZFIN if state.0.is_some() => {
                ZFIN_HEADER.write(port)?;
                break;
            }
            _ if state.0.is_none() => Header::write_zrinit(
                port,
                Encoding::ZHEX,
                Zrinit::CANCRY | Zrinit::CANOVIO | Zrinit::CANFC32,
                0,
            )?,
            _ => (),
        }
    }

    Ok(())
}

/// Writes a ZDATA
fn write_zdata<P, F>(
    port: &mut P,
    file: &mut F,
    header: &Header,
) -> core::result::Result<(), InvalidData>
where
    P: Read + Write,
    F: Reader,
{
    let mut data = [0; SUBPACKET_SIZE as usize];
    let mut offset: u32 = header.count();

    file.seek(offset)?;
    let mut count: u32 = file.read(&mut data)?;
    if count == 0 {
        ZEOF_HEADER.with_count(offset).write(port)?;
        return Ok(());
    }

    ZDATA_HEADER.with_count(offset).write(port)?;
    for _ in 1..SUBPACKET_PER_ACK {
        write_subpacket(
            port,
            Encoding::ZBIN32,
            Packet::ZCRCG,
            &data[..count as usize],
        )?;
        offset += count;

        count = file.read(&mut data)?;
        if count < SUBPACKET_SIZE {
            break;
        }
    }

    write_subpacket(
        port,
        Encoding::ZBIN32,
        Packet::ZCRCW,
        &data[..count as usize],
    )?;
    Ok(())
}

/// Reads a ZDATA packet
fn read_zdata<P, F>(
    encoding: u8,
    count: &mut u32,
    port: &mut P,
    file: &mut F,
) -> core::result::Result<(), InvalidData>
where
    P: Write + Read,
    F: Writer,
{
    let mut buf = RxBuffer::new();

    loop {
        buf.clear();
        let encoding = Encoding::try_from(encoding)?;
        let zcrc = match read_subpacket(port, encoding, &mut buf) {
            Err(_) => {
                ZRPOS_HEADER.with_count(*count).write(port)?;
                return Ok(());
            }
            Ok(zcrc) => zcrc,
        };
        file.write(&buf)?;
        *count += buf.len() as u32;
        match zcrc {
            Packet::ZCRCW => {
                ZACK_HEADER.with_count(*count).write(port)?;
                return Ok(());
            }
            Packet::ZCRCE => return Ok(()),
            Packet::ZCRCQ => {
                ZACK_HEADER.with_count(*count).write(port)?;
            }
            Packet::ZCRCG => log::debug!("ZCRCG"),
        }
    }
}

/// Skips (ZPAD, [ZPAD,] ZDLE) sequence.
fn read_zpad<P>(port: &mut P) -> core::result::Result<(), InvalidData>
where
    P: Read,
{
    if read_byte(port)? != ZPAD {
        return Err(InvalidData);
    }

    let mut b = read_byte(port)?;
    if b == ZPAD {
        b = read_byte(port)?;
    }

    if b == ZDLE {
        return Ok(());
    }

    Err(InvalidData)
}

/// Reads and unescapes a ZMODEM protocol subpacket
fn read_subpacket<P>(
    port: &mut P,
    encoding: Encoding,
    buf: &mut RxBuffer,
) -> core::result::Result<Packet, InvalidData>
where
    P: Read,
{
    let result;

    loop {
        let byte = read_byte(port)?;
        if byte == ZDLE {
            let byte = read_byte(port)?;
            if let Ok(kind) = Packet::try_from(byte) {
                buf.push(kind as u8);
                result = kind;
                break;
            } else {
                buf.push(UNZDLE_TABLE[byte as usize]);
            }
        } else {
            buf.push(byte);
        }
    }

    let crc_len = if encoding == Encoding::ZBIN32 { 4 } else { 2 };
    let mut crc = [0u8; 4];
    for b in crc.iter_mut().take(crc_len) {
        *b = read_byte_unescaped(port)?;
    }
    check_crc(buf, &crc[..crc_len], encoding)?;

    // Pop ZCRC
    buf.pop().unwrap();
    Ok(result)
}

fn write_subpacket<P>(
    port: &mut P,
    encoding: Encoding,
    kind: Packet,
    data: &[u8],
) -> core::result::Result<(), InvalidData>
where
    P: Write,
{
    let kind = kind as u8;
    let mut buf = [0u8; (SUBPACKET_SIZE * 2) as usize];
    let mut len = escape_mem(data, &mut buf[0..(SUBPACKET_SIZE * 2) as usize]);
    port.write_all(&buf[..len]).or(Err(InvalidData))?;
    match encoding {
        Encoding::ZBIN32 => {
            let mut digest = CRC32.digest();
            digest.update(data);
            digest.update(&[kind]);
            len = escape_mem(
                &digest.finalize().to_le_bytes(),
                &mut buf[0..(SUBPACKET_SIZE * 2) as usize],
            )
        }
        Encoding::ZBIN => {
            let mut digest = CRC16.digest();
            digest.update(data);
            digest.update(&[kind]);
            len = escape_mem(
                &digest.finalize().to_be_bytes(),
                &mut buf[0..(SUBPACKET_SIZE * 2) as usize],
            )
        }
        Encoding::ZHEX => {
            unimplemented!()
        }
    };
    port.write_all(&[ZDLE, kind]).or(Err(InvalidData))?;
    port.write_all(&buf[..len]).or(Err(InvalidData))?;
    Ok(())
}

fn check_crc(data: &[u8], crc: &[u8], encoding: Encoding) -> core::result::Result<(), InvalidData> {
    let mut crc2 = [0u8; 4];
    let crc2_len = make_crc(data, &mut crc2, encoding);

    if *crc != crc2[..crc2_len] {
        log::error!("ZCRC mismatch: {:?} != {:?}", crc, &crc2[..crc2_len]);
        return Err(InvalidData);
    }

    Ok(())
}

fn make_crc(data: &[u8], out: &mut [u8], encoding: Encoding) -> usize {
    match encoding {
        Encoding::ZBIN32 => {
            let crc = CRC32.checksum(data).to_le_bytes();
            out[..4].copy_from_slice(&crc[..4]);
            4
        }
        _ => {
            let crc = CRC16.checksum(data).to_be_bytes();
            out[..2].copy_from_slice(&crc[..2]);
            2
        }
    }
}

fn read_byte_unescaped<P>(port: &mut P) -> core::result::Result<u8, InvalidData>
where
    P: Read,
{
    let b = read_byte(port)?;
    Ok(if b == ZDLE {
        UNZDLE_TABLE[read_byte(port)? as usize]
    } else {
        b
    })
}

fn read_byte<P>(port: &mut P) -> core::result::Result<u8, InvalidData>
where
    P: Read,
{
    let mut buf = [0; 1];
    port.read_exact(&mut buf)
        .map(|_| buf[0])
        .or(Err(InvalidData))
}

fn escape_mem(src: &[u8], dst: &mut [u8]) -> usize {
    let mut i = 0;
    for b in src {
        let b_e = ZDLE_TABLE[*b as usize];
        if b_e != *b {
            dst[i] = ZDLE;
            i += 1;
        }
        dst[i] = b_e;
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use crate::{
        read_subpacket, read_zpad, write_subpacket, Encoding, Frame, Header, InvalidData, Packet,
        RxBuffer, XON, ZDLE, ZPAD,
    };

    #[rstest::rstest]
    #[case(Encoding::ZBIN, Frame::ZRQINIT, &[ZPAD, ZDLE, Encoding::ZBIN as u8, 0, 0, 0, 0, 0, 0, 0])]
    #[case(Encoding::ZBIN32, Frame::ZRQINIT, &[ZPAD, ZDLE, Encoding::ZBIN32 as u8, 0, 0, 0, 0, 0, 29, 247, 34, 198])]
    pub fn test_header(#[case] encoding: Encoding, #[case] kind: Frame, #[case] expected: &[u8]) {
        let header = Header::new(encoding, kind).with_flags(&[0; 4]);
        let mut port = vec![];
        header.write(&mut port).unwrap();
        assert_eq!(port, expected);
    }

    #[rstest::rstest]
    #[case(Encoding::ZBIN, Frame::ZRQINIT, &[1, 1, 1, 1], &[ZPAD, ZDLE, Encoding::ZBIN as u8, 0, 1, 1, 1, 1, 98, 148])]
    #[case(Encoding::ZHEX, Frame::ZRQINIT, &[1, 1, 1, 1], &[ZPAD, ZPAD, ZDLE, Encoding::ZHEX as u8, b'0', b'0', b'0', b'1', b'0', b'1', b'0', b'1', b'0', b'1', 54, 50, 57, 52, b'\r', b'\n', XON])]
    pub fn test_header_with_flags(
        #[case] encoding: Encoding,
        #[case] kind: Frame,
        #[case] flags: &[u8; 4],
        #[case] expected: &[u8],
    ) {
        let header = Header::new(encoding, kind).with_flags(flags);
        let mut port = vec![];
        header.write(&mut port).unwrap();
        assert_eq!(port, expected);
    }

    #[rstest::rstest]
    #[case(&[ZPAD, ZDLE], Ok(()))]
    #[case(&[ZPAD, ZPAD, ZDLE], Ok(()))]
    #[case(&[ZDLE], Err(InvalidData))]
    #[case(&[ZPAD, XON], Err(InvalidData))]
    #[case(&[ZPAD, ZPAD, XON], Err(InvalidData))]
    #[case(&[], Err(InvalidData))]
    #[case(&[0; 100], Err(InvalidData))]
    pub fn test_read_zpad(
        #[case] port: &[u8],
        #[case] expected: core::result::Result<(), InvalidData>,
    ) {
        let result = read_zpad(&mut port.to_vec().as_slice());
        if result.is_err() {
            assert_eq!(result.unwrap_err(), expected.unwrap_err());
        }
    }

    #[rstest::rstest]
    #[case(&[Encoding::ZHEX as u8, b'0', b'1', b'0', b'1', b'0', b'2', b'0', b'3', b'0', b'4', b'a', b'7', b'5', b'2'], &Header::new(Encoding::ZHEX, Frame::ZRINIT).with_flags(&[0x1, 0x2, 0x3, 0x4]))]
    #[case(&[Encoding::ZBIN as u8, Frame::ZRINIT as u8, 0xa, 0xb, 0xc, 0xd, 0xa6, 0xcb], &Header::new(Encoding::ZBIN, Frame::ZRINIT).with_flags(&[0xa, 0xb, 0xc, 0xd]))]
    #[case(&[Encoding::ZBIN32 as u8, Frame::ZRINIT as u8, 0xa, 0xb, 0xc, 0xd, 0x99, 0xe2, 0xae, 0x4a], &Header::new(Encoding::ZBIN32, Frame::ZRINIT).with_flags(&[0xa, 0xb, 0xc, 0xd]))]
    #[case(&[Encoding::ZBIN as u8, Frame::ZRINIT as u8, 0xa, ZDLE, b'l', 0xd, ZDLE, b'm', 0x5e, 0x6f], &Header::new(Encoding::ZBIN, Frame::ZRINIT).with_flags(&[0xa, 0x7f, 0xd, 0xff]))]
    pub fn test_header_read(#[case] input: &[u8], #[case] expected: &Header) {
        let input = input.to_vec();
        assert_eq!(&mut Header::read(&mut input.as_slice()).unwrap(), expected);
    }

    #[rstest::rstest]
    #[case(Encoding::ZBIN, Packet::ZCRCE, &[])]
    #[case(Encoding::ZBIN, Packet::ZCRCW, &[0x00])]
    #[case(Encoding::ZBIN32, Packet::ZCRCQ, &[0, 1, 2, 3, 4, 0x60, 0x60])]
    pub fn test_write_read_subpacket(
        #[case] encoding: Encoding,
        #[case] kind: Packet,
        #[case] data: &[u8],
    ) {
        let mut port = vec![];
        write_subpacket(&mut port, encoding, kind, data).unwrap();
        let mut rx_buf = RxBuffer::new();
        assert_eq!(
            read_subpacket(&mut port.as_slice(), encoding, &mut rx_buf).unwrap(),
            kind
        );
        assert_eq!(&rx_buf[..], data);
    }
}
