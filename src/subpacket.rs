// SPDX-License-Identifier: MIT OR Apache-1.0
//! ZMODEM transfer protocol frame

use core::convert::TryFrom;
use std::fmt::{self, Display};
use zerocopy::AsBytes;

#[repr(u8)]
#[allow(clippy::upper_case_acronyms)]
#[derive(AsBytes, Clone, Copy, Debug, PartialEq)]
/// The ZMODEM subpacket type
pub enum Type {
    ZCRCE = 0x68,
    ZCRCG = 0x69,
    ZCRCQ = 0x6a,
    ZCRCW = 0x6b,
}

const TYPES: &[Type] = &[Type::ZCRCE, Type::ZCRCG, Type::ZCRCQ, Type::ZCRCW];

#[derive(Clone, Copy, Debug)]
pub struct InvalidType;

impl TryFrom<u8> for Type {
    type Error = InvalidType;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        TYPES
            .iter()
            .find(|e| value == **e as u8)
            .map_or(Err(InvalidType), |e| Ok(*e))
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:#02x}", *self as u8)
    }
}
