// Copyright 2016 Masaki Hara
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#[cfg(feature = "num-bigint")]
use num_bigint::{BigUint, BigInt};
#[cfg(feature = "bit-vec")]
use bit_vec::BitVec;

use super::Tag;
use super::tags::{TAG_BOOLEAN,TAG_INTEGER,TAG_OCTETSTRING};
use super::tags::{TAG_NULL,TAG_OID,TAG_UTF8STRING,TAG_SEQUENCE,TAG_SET};
use super::tags::{TAG_NUMERICSTRING,TAG_PRINTABLESTRING,TAG_VISIBLESTRING};
use super::models::{ObjectIdentifier,TaggedDerValue};
#[cfg(feature = "chrono")]
use super::models::{UTCTime,GeneralizedTime};

/// Constructs DER-encoded data as `Vec<u8>`.
///
/// This function uses the loan pattern: `callback` is called back with
/// a [`DERWriter`][derwriter], to which the ASN.1 value is written.
///
/// [derwriter]: struct.DERWriter.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_sequence(|writer| {
///         writer.next().write_i64(10);
///         writer.next().write_bool(true);
///     })
/// });
/// assert_eq!(der, vec![48, 6, 2, 1, 10, 1, 1, 255]);
/// ```
pub fn construct_der<F>(callback: F) -> Vec<u8>
        where F: FnOnce(DERWriter) {
    let mut buf = Vec::new();
    {
        let mut writer = DERWriterSeq {
            buf: &mut buf,
        };
        callback(writer.next());
    }
    return buf;
}

/// Constructs DER-encoded sequence of data as `Vec<u8>`.
///
/// This is similar to [`construct_der`][construct_der], but this function
/// accepts more than one ASN.1 values.
///
/// [construct_der]: fn.construct_der.html
///
/// This function uses the loan pattern: `callback` is called back with
/// a [`DERWriterSeq`][derwriterseq], to which the ASN.1 values are written.
///
/// [derwriterseq]: struct.DERWriterSeq.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der_seq(|writer| {
///     writer.next().write_i64(10);
///     writer.next().write_bool(true);
/// });
/// assert_eq!(der, vec![2, 1, 10, 1, 1, 255]);
/// ```
pub fn construct_der_seq<F>(callback: F) -> Vec<u8>
        where F: FnOnce(&mut DERWriterSeq) {
    let mut buf = Vec::new();
    {
        let mut writer = DERWriterSeq {
            buf: &mut buf,
        };
        callback(&mut writer);
    }
    return buf;
}

/// A writer object that accepts an ASN.1 value.
///
/// The two main sources of `DERWriterSeq` are:
///
/// - The [`construct_der`][construct_der] function, the starting point of
///   DER serialization.
/// - The `next` method of [`DERWriterSeq`][derwriterseq].
///
/// [construct_der]: fn.construct_der.html
/// [derwriterseq]: struct.DERWriterSeq.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_i64(10)
/// });
/// assert_eq!(der, vec![2, 1, 10]);
/// ```
#[derive(Debug)]
pub struct DERWriter<'a> {
    buf: &'a mut Vec<u8>,
    implicit_tag: Option<Tag>,
}

impl<'a> DERWriter<'a> {
    fn from_buf(buf: &'a mut Vec<u8>) -> Self {
        return DERWriter {
            buf: buf,
            implicit_tag: None,
        }
    }
    /// Writes BER identifier (tag + primitive/constructed) octets.
    fn write_identifier(&mut self, tag: Tag, pc: PC) {
        let tag = if let Some(tag) = self.implicit_tag { tag } else { tag };
        self.implicit_tag = None;
        let classid = tag.tag_class as u8;
        let pcid = pc as u8;
        if tag.tag_number < 31 {
            self.buf.push(
                (classid << 6) | (pcid << 5) | (tag.tag_number as u8));
            return;
        }
        self.buf.push((classid << 6) | (pcid << 5) | 31);
        let mut shiftnum = 63; // ceil(64 / 7) * 7 - 7
        while (tag.tag_number >> shiftnum) == 0 {
            shiftnum -= 7;
        }
        while shiftnum > 0 {
            self.buf.push(128 | (((tag.tag_number >> shiftnum) & 127) as u8));
            shiftnum -= 7;
        }
        self.buf.push((tag.tag_number & 127) as u8);
    }

    /// Writes BER length octets.
    fn write_length(&mut self, length: usize) {
        let length = length as u64;
        if length < 128 {
            self.buf.push(length as u8);
            return;
        }
        let mut shiftnum = 56; // ceil(64 / 8) * 8 - 8
        while (length >> shiftnum) == 0 {
            shiftnum -= 8;
        }
        self.buf.push(128 | ((shiftnum / 8 + 1) as u8));
        loop {
            self.buf.push((length >> shiftnum) as u8);
            if shiftnum == 0 {
                break;
            }
            shiftnum -= 8;
        }
    }

    /// Deals with unknown length procedures.
    /// This function first marks the current position and
    /// allocates 3 bytes. Then it calls back `callback`.
    /// It then calculates the length and moves the written data
    /// to the actual position. Finally, it writes the length.
    fn with_length<T, F>(&mut self, callback: F) -> T
        where F: FnOnce(&mut Self) -> T {
        let expected_length_length = 3;
        for _ in 0..3 {
            self.buf.push(255);
        }
        let start_pos = self.buf.len();
        let result = callback(self);
        let length = (self.buf.len() - start_pos) as u64;
        let length_length;
        let mut shiftnum = 56; // ceil(64 / 8) * 8 - 8
        if length < 128 {
            length_length = 1;
        } else {
            while (length >> shiftnum) == 0 {
                shiftnum -= 8;
            }
            length_length = shiftnum / 8 + 2;
        }
        let new_start_pos;
        if length_length < expected_length_length {
            let diff = expected_length_length - length_length;
            new_start_pos = start_pos - diff;
            self.buf.drain(new_start_pos .. start_pos);
        } else if length_length > expected_length_length {
            let diff = length_length - expected_length_length;
            new_start_pos = start_pos + diff;
            for _ in 0..diff { self.buf.insert(start_pos, 0); }
        } else {
            new_start_pos = start_pos;
        }
        let mut idx = new_start_pos - length_length;
        if length < 128 {
            self.buf[idx] = length as u8;
        } else {
            self.buf[idx] = 128 | ((shiftnum / 8 + 1) as u8);
            idx += 1;
            loop {
                self.buf[idx] = (length >> shiftnum) as u8;
                idx += 1;
                if shiftnum == 0 {
                    break;
                }
                shiftnum -= 8;
            }
        }
        return result;
    }

    /// Writes `bool` as an ASN.1 BOOLEAN value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bool(true)
    /// });
    /// assert_eq!(der, vec![1, 1, 255]);
    /// ```
    pub fn write_bool(mut self, val: bool) {
        self.write_identifier(TAG_BOOLEAN, PC::Primitive);
        self.write_length(1);
        self.buf.push(if val { 255 } else { 0 });
    }

    /// Writes `i64` as an ASN.1 INTEGER value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_i64(1234567890)
    /// });
    /// assert_eq!(der, vec![2, 4, 73, 150, 2, 210]);
    /// ```
    pub fn write_i64(mut self, val: i64) {
        let mut shiftnum = 56;
        while shiftnum > 0 &&
                (val >> (shiftnum-1) == 0 || val >> (shiftnum-1) == -1) {
            shiftnum -= 8;
        }
        self.write_identifier(TAG_INTEGER, PC::Primitive);
        self.write_length(shiftnum / 8 + 1);
        loop {
            self.buf.push((val >> shiftnum) as u8);
            if shiftnum == 0 {
                break;
            }
            shiftnum -= 8;
        }
    }

    /// Writes `u64` as an ASN.1 INTEGER value.
    pub fn write_u64(mut self, val: u64) {
        let mut shiftnum = 64;
        while shiftnum > 0 && val >> (shiftnum-1) == 0 {
            shiftnum -= 8;
        }
        self.write_identifier(TAG_INTEGER, PC::Primitive);
        self.write_length(shiftnum / 8 + 1);
        if shiftnum == 64 {
            self.buf.push(0);
            shiftnum -= 8;
        }
        loop {
            self.buf.push((val >> shiftnum) as u8);
            if shiftnum == 0 {
                break;
            }
            shiftnum -= 8;
        }
    }

    /// Writes `i32` as an ASN.1 INTEGER value.
    pub fn write_i32(self, val: i32) {
        self.write_i64(val as i64)
    }

    /// Writes `u32` as an ASN.1 INTEGER value.
    pub fn write_u32(self, val: u32) {
        self.write_i64(val as i64)
    }

    /// Writes `i16` as an ASN.1 INTEGER value.
    pub fn write_i16(self, val: i16) {
        self.write_i64(val as i64)
    }

    /// Writes `u16` as an ASN.1 INTEGER value.
    pub fn write_u16(self, val: u16) {
        self.write_i64(val as i64)
    }

    /// Writes `i8` as an ASN.1 INTEGER value.
    pub fn write_i8(self, val: i8) {
        self.write_i64(val as i64)
    }

    /// Writes `u8` as an ASN.1 INTEGER value.
    pub fn write_u8(self, val: u8) {
        self.write_i64(val as i64)
    }

    #[cfg(feature = "num-bigint")]
    /// Writes `BigInt` as an ASN.1 INTEGER value.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate num_bigint;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use num_bigint::BigInt;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bigint(
    ///         &BigInt::parse_bytes(b"1234567890", 10).unwrap())
    /// });
    /// assert_eq!(der, vec![2, 4, 73, 150, 2, 210]);
    /// # }
    /// ```
    ///
    /// # Features
    ///
    /// This method is enabled by `num` feature.
    ///
    /// ```toml
    /// [dependencies]
    /// yasna = { version = "*", features = ["num"] }
    /// ```
    pub fn write_bigint(mut self, val: &BigInt) {
        use num_bigint::Sign;
        self.write_identifier(TAG_INTEGER, PC::Primitive);
        let (sign, mut bytes) = val.to_bytes_le();
        match sign {
            Sign::NoSign => {
                self.write_length(1);
                self.buf.push(0);
            },
            Sign::Plus => {
                let byteslen = bytes.len();
                debug_assert!(bytes[byteslen-1] != 0);
                if bytes[byteslen-1] >= 128 {
                    self.write_length(byteslen+1);
                    self.buf.push(0);
                } else {
                    self.write_length(byteslen);
                }
                bytes.reverse();
                self.buf.extend_from_slice(&bytes);
                return;
            },
            Sign::Minus => {
                let byteslen = bytes.len();
                debug_assert!(bytes[byteslen-1] != 0);
                let mut carry : usize = 1;
                for b in bytes.iter_mut() {
                    let bval = 255 - (*b as usize);
                    *b = (bval + carry) as u8;
                    carry = (bval + carry) >> 8;
                }
                if bytes[byteslen-1] < 128 {
                    self.write_length(byteslen+1);
                    self.buf.push(255);
                } else {
                    self.write_length(byteslen);
                }
                bytes.reverse();
                self.buf.extend_from_slice(&bytes);
                return;
            }
        };
    }

    #[cfg(feature = "num-bigint")]
    /// Writes `BigUint` as an ASN.1 INTEGER value.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate num_bigint;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use num_bigint::BigUint;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_biguint(
    ///         &BigUint::parse_bytes(b"1234567890", 10).unwrap())
    /// });
    /// assert_eq!(der, vec![2, 4, 73, 150, 2, 210]);
    /// # }
    /// ```
    ///
    /// # Features
    ///
    /// This method is enabled by `num` feature.
    ///
    /// ```toml
    /// [dependencies]
    /// yasna = { version = "*", features = ["num"] }
    /// ```
    pub fn write_biguint(mut self, val: &BigUint) {
        self.write_identifier(TAG_INTEGER, PC::Primitive);
        let mut bytes = val.to_bytes_le();
        if &bytes == &[0] {
            self.write_length(1);
            self.buf.push(0);
            return;
        }
        let byteslen = bytes.len();
        debug_assert!(bytes[byteslen-1] != 0);
        if bytes[byteslen-1] >= 128 {
            self.write_length(byteslen+1);
            self.buf.push(0);
        } else {
            self.write_length(byteslen);
        }
        bytes.reverse();
        self.buf.extend_from_slice(&bytes);
    }

    #[cfg(feature = "bit-vec")]
    /// Writes `BitVec` as an ASN.1 BITSTRING value.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate bit_vec;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use bit_vec::BitVec;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bitvec(&
    ///         [1, 1, 0, 0, 1, 1, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1,
    ///             0, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 1, 1]
    ///         .iter().map(|&i| i != 0).collect())
    /// });
    /// assert_eq!(&der, &[3, 5, 3, 206, 213, 116, 24]);
    /// # }
    /// ```
    ///
    /// # Features
    ///
    /// This method is enabled by `bit-vec` feature.
    ///
    /// ```toml
    /// [dependencies]
    /// yasna = { version = "*", features = ["bit-vec"] }
    /// ```
    pub fn write_bitvec(mut self, bitvec: &BitVec) {
        use super::tags::TAG_BITSTRING;
        self.write_identifier(TAG_BITSTRING, PC::Primitive);
        let len = bitvec.len();
        let bytes = bitvec.to_bytes();
        debug_assert!(len <= 8 * bytes.len());
        debug_assert!(8 * bytes.len() < len + 8);
        self.write_length(1 + bytes.len());
        self.buf.push((8 * bytes.len() - len) as u8);
        self.buf.extend_from_slice(&bytes);
    }

    /// Writes `&[u8]` as an ASN.1 OCTETSTRING value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bytes(b"Hello!")
    /// });
    /// assert_eq!(der, vec![4, 6, 72, 101, 108, 108, 111, 33]);
    /// ```
    pub fn write_bytes(mut self, bytes: &[u8]) {
        self.write_identifier(TAG_OCTETSTRING, PC::Primitive);
        self.write_length(bytes.len());
        self.buf.extend_from_slice(bytes);
    }

    /// Writes `&str` as an ASN.1 UTF8String value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_utf8_string("Hello!")
    /// });
    /// assert_eq!(der, vec![12, 6, 72, 101, 108, 108, 111, 33]);
    /// ```
    pub fn write_utf8_string(mut self, string: &str) {
        self.write_identifier(TAG_UTF8STRING, PC::Primitive);
        self.write_length(string.len());
        self.buf.extend_from_slice(string.as_bytes());
    }

    /// Writes the ASN.1 NULL value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_null()
    /// });
    /// assert_eq!(der, vec![5, 0]);
    /// ```
    pub fn write_null(mut self) {
        self.write_identifier(TAG_NULL, PC::Primitive);
        self.write_length(0);
    }

    /// Writes an ASN.1 object identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// use yasna::models::ObjectIdentifier;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_oid(&ObjectIdentifier::from_slice(
    ///         &[1, 2, 840, 113549, 1, 1]))
    /// });
    /// assert_eq!(&der, &[6, 8, 42, 134, 72, 134, 247, 13, 1, 1]);
    /// ```
    ///
    /// # Panics
    ///
    /// It panics when the OID cannot be canonically encoded in BER.
    pub fn write_oid(mut self, oid: &ObjectIdentifier) {
        assert!(oid.components().len() >= 2, "Invalid OID: too short");
        let id0 = oid.components()[0];
        let id1 = oid.components()[1];
        assert!(
            (id0 < 3) && (id1 < 18446744073709551535) &&
            (id0 >= 2 || id1 < 40),
            "Invalid OID {{{} {} ...}}", id0, id1);
        let subid0 = id0 * 40 + id1;
        let mut length = 0;
        for i in 1..oid.components().len() {
            let mut subid = if i == 1 {
                subid0
            } else {
                oid.components()[i]
            } | 1;
            while subid > 0 {
                length += 1;
                subid >>= 7;
            }
        }
        self.write_identifier(TAG_OID, PC::Primitive);
        self.write_length(length);
        for i in 1..oid.components().len() {
            let subid = if i == 1 {
                subid0
            } else {
                oid.components()[i]
            };
            let mut shiftnum = 63; // ceil(64 / 7) * 7 - 7
            while ((subid|1) >> shiftnum) == 0 {
                shiftnum -= 7;
            }
            while shiftnum > 0 {
                self.buf.push(128 | ((((subid|1) >> shiftnum) & 127) as u8));
                shiftnum -= 7;
            }
            self.buf.push((subid & 127) as u8);
        }
    }

    /// Writes an ASN.1 UTF8String.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_utf8string("gnaw ροκανίζω 𪘂る")
    /// });
    /// assert_eq!(&der, &[
    ///     12, 29, 103, 110, 97, 119, 32, 207, 129, 206, 191, 206,
    ///     186, 206, 177, 206, 189, 206, 175, 206, 182, 207,
    ///     137, 32, 240, 170, 152, 130, 227, 130, 139]);
    /// ```
    pub fn write_utf8string(self, string: &str) {
        self.write_tagged_implicit(TAG_UTF8STRING, |writer| {
            writer.write_bytes(string.as_bytes())
        })
    }

    /// Writes ASN.1 SEQUENCE.
    ///
    /// This function uses the loan pattern: `callback` is called back with
    /// a [`DERWriterSeq`][derwriterseq], to which the contents of the
    /// SEQUENCE is written.
    ///
    /// [derwriterseq]: struct.DERWriterSeq.html
    ///
    /// This is equivalent to `write_sequence_of` in behaviors.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_sequence(|writer| {
    ///         writer.next().write_i64(10);
    ///         writer.next().write_bool(true);
    ///     })
    /// });
    /// assert_eq!(der, vec![48, 6, 2, 1, 10, 1, 1, 255]);
    /// ```
    pub fn write_sequence<T, F>(mut self, callback: F) -> T
        where F: FnOnce(&mut DERWriterSeq) -> T {
        self.write_identifier(TAG_SEQUENCE, PC::Constructed);
        return self.with_length(|writer| {
            callback(&mut DERWriterSeq {
                buf: writer.buf,
            })
        });
    }

    /// Writes ASN.1 SEQUENCE OF.
    ///
    /// This function uses the loan pattern: `callback` is called back with
    /// a [`DERWriterSeq`][derwriterseq], to which the contents of the
    /// SEQUENCE OF are written.
    ///
    /// [derwriterseq]: struct.DERWriterSeq.html
    ///
    /// This is equivalent to `write_sequence` in behaviors.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_sequence_of(|writer| {
    ///         for &i in &[10, -129] {
    ///             writer.next().write_i64(i);
    ///         }
    ///     })
    /// });
    /// assert_eq!(der, vec![48, 7, 2, 1, 10, 2, 2, 255, 127]);
    /// ```
    pub fn write_sequence_of<T, F>(self, callback: F) -> T
        where F: FnOnce(&mut DERWriterSeq) -> T {
        self.write_sequence(callback)
    }

    /// Writes ASN.1 SET.
    ///
    /// This function uses the loan pattern: `callback` is called back with
    /// a [`DERWriterSet`][derwriterset], to which the contents of the
    /// SET are written.
    ///
    /// [derwriterset]: struct.DERWriterSet.html
    ///
    /// For SET OF values, use `write_set_of` instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_set(|writer| {
    ///         writer.next().write_i64(10);
    ///         writer.next().write_bool(true);
    ///     })
    /// });
    /// assert_eq!(der, vec![49, 6, 1, 1, 255, 2, 1, 10]);
    /// ```
    pub fn write_set<T, F>(mut self, callback: F) -> T
        where F: FnOnce(&mut DERWriterSet) -> T {
        let mut bufs = Vec::new();
        let result = callback(&mut DERWriterSet {
            bufs: &mut bufs,
        });
        for buf in bufs.iter() {
            assert!(buf.len() > 0, "Empty output in write_set()");
        }
        bufs.sort_by(|buf0, buf1| {
            let buf00 = buf0[0] & 223;
            let buf10 = buf1[0] & 223;
            if buf00 != buf10 || (buf0[0] & 31) != 31 {
                return buf00.cmp(&buf10);
            }
            let len0 = buf0[1..].iter().position(|x| x & 128 == 0).unwrap();
            let len1 = buf1[1..].iter().position(|x| x & 128 == 0).unwrap();
            if len0 != len1 {
                return len0.cmp(&len1);
            }
            return buf0[1..].cmp(&buf1[1..]);
        });
        // let bufs_len = bufs.iter().map(|buf| buf.len()).sum();
        let bufs_len = bufs.iter().map(|buf| buf.len()).fold(0, |x, y| x + y);
        self.write_identifier(TAG_SET, PC::Constructed);
        self.write_length(bufs_len);
        for buf in bufs.iter() {
            self.buf.extend_from_slice(buf);
        }
        return result;
    }

    /// Writes ASN.1 SET OF.
    ///
    /// This function uses the loan pattern: `callback` is called back with
    /// a [`DERWriterSet`][derwriterset], to which the contents of the
    /// SET OF are written.
    ///
    /// [derwriterset]: struct.DERWriterSet.html
    ///
    /// For SET values, use `write_set` instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_set_of(|writer| {
    ///         for &i in &[10, -129] {
    ///             writer.next().write_i64(i);
    ///         }
    ///     })
    /// });
    /// assert_eq!(der, vec![49, 7, 2, 1, 10, 2, 2, 255, 127]);
    /// ```
    pub fn write_set_of<T, F>(mut self, callback: F) -> T
        where F: FnOnce(&mut DERWriterSet) -> T {
        let mut bufs = Vec::new();
        let result = callback(&mut DERWriterSet {
            bufs: &mut bufs,
        });
        for buf in bufs.iter() {
            assert!(buf.len() > 0, "Empty output in write_set_of()");
        }
        bufs.sort();
        // let bufs_len = bufs.iter().map(|buf| buf.len()).sum();
        let bufs_len = bufs.iter().map(|buf| buf.len()).fold(0, |x, y| x + y);
        self.write_identifier(TAG_SET, PC::Constructed);
        self.write_length(bufs_len);
        for buf in bufs.iter() {
            self.buf.extend_from_slice(buf);
        }
        return result;
    }

    /// Writes an ASN.1 NumericString.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_numeric_string("128 256")
    /// });
    /// assert_eq!(&der, &[18, 7, 49, 50, 56, 32, 50, 53, 54]);
    /// ```
    pub fn write_numeric_string(self, string: &str) {
        let bytes = string.as_bytes();
        for &byte in bytes {
            assert!(byte == b' ' || (b'0' <= byte && byte <= b'9'),
                "Invalid NumericString: {:?} appeared", byte);
        }
        self.write_tagged_implicit(TAG_NUMERICSTRING, |writer| {
            writer.write_bytes(bytes)
        });
    }

    /// Writes an ASN.1 PrintableString.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_printable_string("Co., Ltd.")
    /// });
    /// assert_eq!(&der, &[19, 9, 67, 111, 46, 44, 32, 76, 116, 100, 46]);
    /// ```
    pub fn write_printable_string(self, string: &str) {
        let bytes = string.as_bytes();
        for &byte in bytes {
            assert!(
                byte == b' ' ||
                (b'\'' <= byte && byte <= b':' && byte != b'*') ||
                byte == b'=' ||
                (b'A' <= byte && byte <= b'Z') ||
                (b'a' <= byte && byte <= b'z'),
                "Invalid PrintableString: {:?} appeared", byte);
        }
        self.write_tagged_implicit(TAG_PRINTABLESTRING, |writer| {
            writer.write_bytes(bytes)
        });
    }

    #[cfg(feature = "chrono")]
    /// Writes an ASN.1 UTCTime.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate chrono;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use yasna::models::UTCTime;
    /// use chrono::{Utc,TimeZone};
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_utctime(
    ///         &UTCTime::from_datetime(&Utc.timestamp(378820800, 0)))
    /// });
    /// assert_eq!(&der, &[
    ///     23, 13, 56, 50, 48, 49, 48, 50, 49, 50, 48, 48, 48, 48, 90]);
    /// # }
    /// ```
    ///
    /// # Features
    ///
    /// This method is enabled by `chrono` feature.
    ///
    /// ```toml
    /// [dependencies]
    /// yasna = { version = "*", features = ["chrono"] }
    /// ```
    pub fn write_utctime(self, datetime: &UTCTime) {
        use super::tags::TAG_UTCTIME;
        self.write_tagged_implicit(TAG_UTCTIME, |writer| {
            writer.write_bytes(&datetime.to_bytes())
        });
    }

    #[cfg(feature = "chrono")]
    /// Writes an ASN.1 GeneralizedTime.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate chrono;
    /// # extern crate yasna;
    /// # fn main() {
    /// use yasna;
    /// use yasna::models::GeneralizedTime;
    /// use chrono::{Utc,TimeZone};
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_generalized_time(
    ///         &GeneralizedTime::from_datetime(
    ///             &Utc.timestamp(500159309, 724_000_000)))
    /// });
    /// assert_eq!(&der, &[
    ///     24, 19, 49, 57, 56, 53, 49, 49, 48, 54, 50,
    ///     49, 48, 56, 50, 57, 46, 55, 50, 52, 90]);
    /// # }
    /// ```
    ///
    /// # Features
    ///
    /// This method is enabled by `chrono` feature.
    ///
    /// ```toml
    /// [dependencies]
    /// yasna = { version = "*", features = ["chrono"] }
    /// ```
    pub fn write_generalized_time(self, datetime: &GeneralizedTime) {
        use super::tags::TAG_GENERALIZEDTIME;
        self.write_tagged_implicit(TAG_GENERALIZEDTIME, |writer| {
            writer.write_bytes(&datetime.to_bytes())
        });
    }

    /// Writes an ASN.1 VisibleString.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_visible_string("Hi!")
    /// });
    /// assert_eq!(&der, &[26, 3, 72, 105, 33]);
    /// ```
    pub fn write_visible_string(self, string: &str) {
        let bytes = string.as_bytes();
        for &byte in bytes {
            assert!(b' ' <= byte && byte <= b'~',
                "Invalid VisibleString: {:?} appeared", byte);
        }
        self.write_tagged_implicit(TAG_VISIBLESTRING, |writer| {
            writer.write_bytes(bytes)
        });
    }

    /// Writes a (explicitly) tagged value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna::{self,Tag};
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_tagged(Tag::context(3), |writer| {
    ///         writer.write_i64(10)
    ///     })
    /// });
    /// assert_eq!(der, vec![163, 3, 2, 1, 10]);
    /// ```
    ///
    /// Note: you can achieve the same using `write_tagged_implicit`:
    ///
    /// ```
    /// use yasna::{self,Tag};
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_tagged_implicit(Tag::context(3), |writer| {
    ///         writer.write_sequence(|writer| {
    ///             let writer = writer.next();
    ///             writer.write_i64(10)
    ///         })
    ///     })
    /// });
    /// assert_eq!(der, vec![163, 3, 2, 1, 10]);
    /// ```
    pub fn write_tagged<T, F>(mut self, tag: Tag, callback: F) -> T
        where F: FnOnce(DERWriter) -> T {
        self.write_identifier(tag, PC::Constructed);
        return self.with_length(|writer| {
            callback(DERWriter::from_buf(writer.buf))
        });
    }

    /// Writes an implicitly tagged value.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna::{self,Tag};
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_tagged_implicit(Tag::context(3), |writer| {
    ///         writer.write_i64(10)
    ///     })
    /// });
    /// assert_eq!(der, vec![131, 1, 10]);
    /// ```
    pub fn write_tagged_implicit<T, F>
        (mut self, tag: Tag, callback: F) -> T
        where F: FnOnce(DERWriter) -> T {
        let tag = if let Some(tag) = self.implicit_tag { tag } else { tag };
        self.implicit_tag = None;
        let mut writer = DERWriter::from_buf(self.buf);
        writer.implicit_tag = Some(tag);
        return callback(writer);
    }

    /// Writes the arbitrary tagged DER value in `der`.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// use yasna::models::TaggedDerValue;
    /// use yasna::tags::TAG_OCTETSTRING;
    /// let tagged_der_value = TaggedDerValue::from_tag_and_bytes(TAG_OCTETSTRING, b"Hello!".to_vec());
    /// let der1 = yasna::construct_der(|writer| {
    ///     writer.write_tagged_der(&tagged_der_value)
    /// });
    /// let der2 = yasna::construct_der(|writer| {
    ///     writer.write_bytes(b"Hello!")
    /// });
    /// assert_eq!(der1, der2);
    /// ```
    pub fn write_tagged_der(mut self, der: &TaggedDerValue) {
        let pc = match der.tag() {
            TAG_SEQUENCE | TAG_SET => PC::Constructed,
            _ => PC::Primitive,
        };
        self.write_identifier(der.tag(), pc);
        self.write_length(der.value().len());
        self.buf.extend_from_slice(der.value());
    }

    /// Writes `&[u8]` into the DER output buffer directly. Properly encoded tag
    /// and length must be included at the start of the passed buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use yasna;
    /// let raw_der = yasna::construct_der(|writer| {
    ///     writer.write_der(b"\x04\x06Hello!")
    /// });
    /// let der = yasna::construct_der(|writer| {
    ///     writer.write_bytes(b"Hello!")
    /// });
    /// assert_eq!(raw_der, der);
    /// ```
    pub fn write_der(self, der: &[u8]) {
        self.buf.extend_from_slice(der);
    }
}

/// A writer object that accepts ASN.1 values.
///
/// The main source of this object is the `write_sequence` method from
/// [`DERWriter`][derwriter].
///
/// [derwriter]: struct.DERWriter.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_sequence(|writer : &mut yasna::DERWriterSeq| {
///         writer.next().write_i64(10);
///         writer.next().write_bool(true);
///     })
/// });
/// assert_eq!(der, vec![48, 6, 2, 1, 10, 1, 1, 255]);
/// ```
#[derive(Debug)]
pub struct DERWriterSeq<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> DERWriterSeq<'a> {
    /// Generates a new [`DERWriter`][derwriter].
    ///
    /// [derwriter]: struct.DERWriter.html
    pub fn next<'b>(&'b mut self) -> DERWriter<'b> {
        return DERWriter::from_buf(self.buf);
    }
}

/// A writer object that accepts ASN.1 values.
///
/// The main source of this object is the `write_set` method from
/// [`DERWriter`][derwriter].
///
/// [derwriter]: struct.DERWriter.html
///
/// # Examples
///
/// ```
/// use yasna;
/// let der = yasna::construct_der(|writer| {
///     writer.write_set(|writer : &mut yasna::DERWriterSet| {
///         writer.next().write_i64(10);
///         writer.next().write_bool(true);
///     })
/// });
/// assert_eq!(der, vec![49, 6, 1, 1, 255, 2, 1, 10]);
/// ```
#[derive(Debug)]
pub struct DERWriterSet<'a> {
    bufs: &'a mut Vec<Vec<u8>>,
}

impl<'a> DERWriterSet<'a> {
    /// Generates a new [`DERWriter`][derwriter].
    ///
    /// [derwriter]: struct.DERWriter.html
    pub fn next<'b>(&'b mut self) -> DERWriter<'b> {
        self.bufs.push(Vec::new());
        return DERWriter::from_buf(self.bufs.last_mut().unwrap());
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum PC {
    Primitive = 0, Constructed = 1,
}

#[cfg(test)]
mod tests;
