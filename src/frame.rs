// SPDX-License-Identifier: MIT OR Apache-2.0
//! ZMODEM transfer protocol frame

use crate::consts::*;
use crate::zerocopy::AsBytes;
use core::convert::TryFrom;
use std::fmt::{self, Display};

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(AsBytes, Clone, Copy, Debug, PartialEq)]
/// The ZMODEM frame type
pub enum Encoding {
    ZBIN = 0x41,
    ZHEX = 0x42,
    ZBIN32 = 0x43,
}

const ENCODINGS: &[Encoding] = &[Encoding::ZBIN, Encoding::ZHEX, Encoding::ZBIN32];

#[derive(Clone, Copy, Debug)]
pub struct InvalidEncoding;

impl TryFrom<u8> for Encoding {
    type Error = InvalidEncoding;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        ENCODINGS
            .iter()
            .find(|e| value == **e as u8)
            .map_or(Err(InvalidEncoding), |e| Ok(*e))
    }
}

impl Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#02x}", *self as u8)
    }
}

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(AsBytes, Clone, Copy, Debug, PartialEq)]
/// The ZMODEM frame type
pub enum Type {
    /// Request receive init
    ZRQINIT = 0,
    /// Receive init
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

const TYPES: &[Type] = &[
    Type::ZRQINIT,
    Type::ZRINIT,
    Type::ZSINIT,
    Type::ZACK,
    Type::ZFILE,
    Type::ZSKIP,
    Type::ZNAK,
    Type::ZABORT,
    Type::ZFIN,
    Type::ZRPOS,
    Type::ZDATA,
    Type::ZEOF,
    Type::ZFERR,
    Type::ZCRC,
    Type::ZCHALLENGE,
    Type::ZCOMPL,
    Type::ZCAN,
    Type::ZFREECNT,
    Type::ZCOMMAND,
    Type::ZSTDERR,
];

#[derive(Clone, Copy, Debug)]
pub struct InvalidType;

impl TryFrom<u8> for Type {
    type Error = InvalidType;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        TYPES
            .iter()
            .find(|t| value == **t as u8)
            .map_or(Err(InvalidType), |t| Ok(*t))
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#02x}", *self as u8)
    }
}

pub fn escape_u8(value: u8) -> Option<[u8; 2]> {
    Some(match value {
        0xFF => [ZLDE, ESC_FF],
        0x7F => [ZLDE, ESC_7F],
        0x10 | 0x90 | 0x11 | 0x91 | 0x13 | 0x93 => [ZLDE, value ^ 0x40],
        ZLDE => [ZLDE, ZLDEE],
        _ => return None,
    })
}

pub fn escape_u8_array(src: &[u8], dst: &mut Vec<u8>) {
    for value in src {
        if let Some(value) = escape_u8(*value) {
            dst.extend_from_slice(&value);
        } else {
            dst.push(*value);
        }
    }
}

#[repr(C)]
#[derive(AsBytes, Clone, Copy, Debug)]
pub struct Header {
    encoding: Encoding,
    frame_type: Type,
    flags: [u8; 4],
}

impl Header {
    pub fn new(encoding: Encoding, frame_type: Type, flags: &[u8; 4]) -> Header {
        Header {
            encoding,
            frame_type,
            flags: *flags,
        }
    }

    pub fn new_count(encoding: Encoding, frame_type: Type, count: u32) -> Header {
        Header {
            encoding,
            frame_type,
            flags: count.to_le_bytes(),
        }
    }

    pub fn get_count(&self) -> u32 {
        u32::from_le_bytes(self.flags)
    }

    pub fn frame_type(&self) -> Type {
        self.frame_type
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:8} {}", self.encoding, self.frame_type)
    }
}

pub fn new_frame(header: &Header, out: &mut Vec<u8>) {
    out.push(ZPAD);
    if header.encoding == Encoding::ZHEX {
        out.push(ZPAD);
    }

    out.push(ZLDE);
    out.extend_from_slice(header.as_bytes());

    // Skips ZPAD and encoding:
    match header.encoding {
        Encoding::ZBIN32 => out.extend_from_slice(&CRC32.checksum(&out[3..]).to_le_bytes()),
        Encoding::ZHEX => out.extend_from_slice(&CRC16.checksum(&out[4..]).to_be_bytes()),
        _ => out.extend_from_slice(&CRC16.checksum(&out[3..]).to_be_bytes()),
    };

    // Skips ZPAD and encoding:
    if header.encoding == Encoding::ZHEX {
        let hex = hex::encode(&out[4..]);
        out.truncate(4);
        out.extend_from_slice(hex.as_bytes());
    }

    escape_u8_array(&out.drain(3..).collect::<Vec<_>>(), out);

    if header.encoding == Encoding::ZHEX {
        // Add trailing CRLF for ZHEX transfer:
        out.extend_from_slice(b"\r\n");

        if header.frame_type != Type::ZACK && header.frame_type != Type::ZFIN {
            out.push(XON);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::frame::*;

    #[rstest::rstest]
    #[case(Encoding::ZBIN, Type::ZRQINIT, &[ZPAD, ZLDE, Encoding::ZBIN as u8, 0, 0, 0, 0, 0, 0, 0])]
    #[case(Encoding::ZBIN32, Type::ZRQINIT, &[ZPAD, ZLDE, Encoding::ZBIN32 as u8, 0, 0, 0, 0, 0, 29, 247, 34, 198])]
    fn test_header(#[case] encoding: Encoding, #[case] frame_type: Type, #[case] expected: &[u8]) {
        let header = Header::new(encoding, frame_type, &[0; 4]);

        let mut frame = vec![];
        new_frame(&header, &mut frame);

        assert_eq!(frame, expected);
    }
    #[rstest::rstest]
    #[case(Encoding::ZBIN, Type::ZRQINIT, &[1, 1, 1, 1], &[ZPAD, ZLDE, Encoding::ZBIN as u8, 0, 1, 1, 1, 1, 98, 148])]
    #[case(Encoding::ZHEX, Type::ZRQINIT, &[1, 1, 1, 1], &[ZPAD, ZPAD, ZLDE, Encoding::ZHEX as u8, b'0', b'0', b'0', b'1', b'0', b'1', b'0', b'1', b'0', b'1', 54, 50, 57, 52, b'\r', b'\n', XON])]
    fn test_header_with_flags(
        #[case] encoding: Encoding,
        #[case] frame_type: Type,
        #[case] flags: &[u8; 4],
        #[case] expected: &[u8],
    ) {
        let header = Header::new(encoding, frame_type, flags);

        let mut frame = vec![];
        new_frame(&header, &mut frame);

        assert_eq!(frame, expected);
    }
}
