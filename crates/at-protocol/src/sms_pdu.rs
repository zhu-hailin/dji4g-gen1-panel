//! SMS PDU encode/decode (3GPP TS 23.040), pure logic with no IO.
//!
//! Candidate implementation based on the research document (appendix A, §6); 实机未验证. The
//! caller is responsible for transaction framing: this module never touches the serial port and
//! never decides whether a send is authorized.

use std::fmt;

use dji4g_domain::{SmsConcatReference, SmsEncoding, SmsMultipartInfo};

/// Maximum accepted hex input length for one PDU.
const MAX_PDU_HEX_CHARS: usize = 4096;
/// Maximum recipient length in digits for an explicit international number.
const MAX_RECIPIENT_DIGITS: usize = 15;
/// Maximum user data bytes for a single UCS-2 SMS-SUBMIT.
const MAX_UCS2_BODY_BYTES: usize = 140;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmsPduError {
    TooLong,
    Csv,
    Shape,
    Number,
    Range,
    Encoding,
    Multipart,
}

/// One decoded SMS-DELIVER.
///
/// The sender and body are private-format message content; `Debug` redacts both. The timestamp
/// is the service-centre timestamp as reported in the PDU, never system time.
#[derive(Clone, Eq, PartialEq)]
pub struct DecodedSms {
    pub sender: String,
    pub timestamp: Option<String>,
    pub body: String,
    pub encoding: SmsEncoding,
    pub multipart: Option<SmsMultipartInfo>,
    /// The PDU itself carries no read state; storage status (`CMGL`/`CMGR`) is the only
    /// evidence, so this stays `None` here.
    pub read: Option<bool>,
}

impl fmt::Debug for DecodedSms {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedSms")
            .field("sender", &"[REDACTED]")
            .field("timestamp", &self.timestamp)
            .field("body", &"[REDACTED]")
            .field("encoding", &self.encoding)
            .field("multipart", &self.multipart)
            .field("read", &self.read)
            .finish()
    }
}

/// One UCS-2 SMS-SUBMIT PDU. The payload hex is only exposed for the send transaction.
pub struct EncodedSubmit {
    hex: String,
    pub tpdu_octets: usize,
}

impl EncodedSubmit {
    /// Only for the AT actor holding a single confirmed send; never pass this to a log.
    #[must_use]
    pub fn expose_for_confirmed_send(&self) -> &str {
        &self.hex
    }
}

impl fmt::Debug for EncodedSubmit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedSubmit")
            .field("payload", &"[REDACTED]")
            .field("tpdu_octets", &self.tpdu_octets)
            .finish()
    }
}

/// Decode one SMS-DELIVER PDU given as an ASCII hex string.
///
/// The whole PDU is atomic: any malformed shape, number, timestamp or encoding rejects the
/// message instead of returning partially guessed content (research §6.2).
pub fn decode_deliver_pdu(hex: &str) -> Result<DecodedSms, SmsPduError> {
    let bytes = hex_to_bytes(hex)?;
    let mut cursor = Cursor::new(&bytes);

    let smsc_length = usize::from(cursor.take_u8()?);
    cursor.take(smsc_length)?;

    let first_octet = cursor.take_u8()?;
    if first_octet & 0x03 != 0x00 {
        return Err(SmsPduError::Shape);
    }
    let has_user_data_header = first_octet & 0x40 != 0;

    let address_length = usize::from(cursor.take_u8()?);
    let type_of_address = cursor.take_u8()?;
    let address_octets = cursor.take(address_length.div_ceil(2))?;
    let sender = decode_sender(address_length, type_of_address, address_octets)?;

    cursor.take_u8()?; // TP-PID has no modeled interpretation yet.
    let data_coding_scheme = cursor.take_u8()?;
    let encoding = encoding_from_dcs(data_coding_scheme);

    let timestamp = decode_scts(cursor.take(7)?)?;
    let user_data_length = usize::from(cursor.take_u8()?);
    let user_data = cursor.remaining();

    let (body, multipart) = match encoding {
        SmsEncoding::Gsm7 => {
            decode_gsm7_user_data(user_data, user_data_length, has_user_data_header)?
        }
        SmsEncoding::Ucs2 => {
            decode_ucs2_user_data(user_data, user_data_length, has_user_data_header)?
        }
        // A DCS this codec does not model: the body stays unset, never guessed.
        SmsEncoding::Other => (String::new(), None),
    };

    Ok(DecodedSms {
        sender,
        timestamp: Some(timestamp),
        body,
        encoding,
        multipart,
        read: None,
    })
}

/// Build a single-fragment UCS-2 SMS-SUBMIT PDU (no SMSC, no validity period, no delivery
/// report). Recipients are explicit international numbers only; the body is BMP only and must
/// fit 140 user-data octets.
pub fn build_ucs2_submit(recipient: &str, text: &str) -> Result<EncodedSubmit, SmsPduError> {
    validate_sms_recipient(recipient)?;
    let digits = &recipient[1..];
    build_ucs2_submit_validated(digits, text)
}

/// Validate an explicit international recipient without guessing or normalizing its prefix.
pub fn validate_sms_recipient(recipient: &str) -> Result<(), SmsPduError> {
    let digits = recipient.strip_prefix('+').ok_or(SmsPduError::Number)?;
    if digits.is_empty()
        || digits.len() > MAX_RECIPIENT_DIGITS
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SmsPduError::Number);
    }
    Ok(())
}

fn build_ucs2_submit_validated(digits: &str, text: &str) -> Result<EncodedSubmit, SmsPduError> {
    if text.is_empty() {
        return Err(SmsPduError::Range);
    }

    let mut body = Vec::new();
    for character in text.chars() {
        let unit = u32::from(character);
        if unit > 0xFFFF {
            return Err(SmsPduError::Encoding);
        }
        body.extend_from_slice(&(unit as u16).to_be_bytes());
        if body.len() > MAX_UCS2_BODY_BYTES {
            return Err(SmsPduError::TooLong);
        }
    }

    // SCA length 0 / SMS-SUBMIT (no validity period) / message reference / DA / international
    // TOA (research appendix A).
    let digit_bytes = digits.as_bytes();
    let mut pdu = Vec::with_capacity(16 + body.len());
    pdu.extend_from_slice(&[0x00, 0x01, 0x00, digit_bytes.len() as u8, 0x91]);
    for pair in digit_bytes.chunks(2) {
        let low = pair[0] - b'0';
        let high = pair.get(1).map_or(0x0F, |value| value - b'0');
        pdu.push(low | (high << 4));
    }
    pdu.extend_from_slice(&[0x00, 0x08, body.len() as u8]); // TP-PID / UCS-2 DCS / TP-UDL
    pdu.extend_from_slice(&body);

    let tpdu_octets = pdu.len() - 1; // The SCA length octet and empty SCA are not TPDU.
    let hex = pdu
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("");
    Ok(EncodedSubmit { hex, tpdu_octets })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], SmsPduError> {
        let end = self.position.checked_add(count).ok_or(SmsPduError::Shape)?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(SmsPduError::Shape)?;
        self.position = end;
        Ok(slice)
    }

    fn take_u8(&mut self) -> Result<u8, SmsPduError> {
        Ok(self.take(1)?[0])
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.position..]
    }
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, SmsPduError> {
    if hex.len() > MAX_PDU_HEX_CHARS {
        return Err(SmsPduError::TooLong);
    }
    if hex.len() % 2 != 0 {
        return Err(SmsPduError::Shape);
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| Ok(hex_nibble(pair[0])? << 4 | hex_nibble(pair[1])?))
        .collect()
}

fn hex_nibble(byte: u8) -> Result<u8, SmsPduError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(SmsPduError::Shape),
    }
}

/// General data coding uses alphabet bits 3..2; the class-present bit carries no alphabet
/// meaning and the low class bits are ignored here. The 0xF0 group is the default alphabet.
fn encoding_from_dcs(dcs: u8) -> SmsEncoding {
    match dcs & 0xF0 {
        // General data coding without and with the class-present bit; the class bits
        // themselves carry no alphabet meaning.
        0x00 | 0x10 => match (dcs >> 2) & 0x03 {
            0b00 => SmsEncoding::Gsm7,
            0b10 => SmsEncoding::Ucs2,
            _ => SmsEncoding::Other,
        },
        0xF0 => SmsEncoding::Gsm7,
        _ => SmsEncoding::Other,
    }
}

/// Raw address digits with `+` only for the international type-of-number (0x91); the unknown
/// type (0x81) and the national type keep the reported digits unprefixed.
fn decode_sender(length: usize, type_of_address: u8, octets: &[u8]) -> Result<String, SmsPduError> {
    if type_of_address & 0x80 == 0 {
        return Err(SmsPduError::Number);
    }
    let mut digits = String::with_capacity(length + 1);
    for index in 0..length {
        let byte = octets[index / 2];
        let nibble = if index % 2 == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        };
        if nibble > 9 {
            return Err(SmsPduError::Number);
        }
        digits.push(char::from(b'0' + nibble));
    }
    match (type_of_address >> 4) & 0x07 {
        0b001 => Ok(format!("+{digits}")),
        0b000 | 0b010 => Ok(digits),
        _ => Err(SmsPduError::Number),
    }
}

/// Swapped-BCD service-centre timestamp as `YYMMDDHHMMSS+ZZ`, where the trailing two digits are
/// the time-zone offset in quarters of an hour and the sign comes from bit 3 of the tens digit.
fn decode_scts(octets: &[u8]) -> Result<String, SmsPduError> {
    let mut output = String::with_capacity(15);
    for &byte in &octets[..6] {
        let low = byte & 0x0F;
        let high = byte >> 4;
        if low > 9 || high > 9 {
            return Err(SmsPduError::Shape);
        }
        output.push(char::from(b'0' + low));
        output.push(char::from(b'0' + high));
    }
    // The time-zone octet is swapped BCD too: the low nibble is the tens digit and carries the
    // sign in bit 3; the high nibble is the units digit.
    let units = octets[6] >> 4;
    let mut tens = octets[6] & 0x0F;
    if units > 9 {
        return Err(SmsPduError::Shape);
    }
    output.push(if tens & 0x08 != 0 { '-' } else { '+' });
    tens &= 0x07;
    output.push(char::from(b'0' + tens));
    output.push(char::from(b'0' + units));
    Ok(output)
}

/// Parse the UDH starting at the first user-data octet; returns the concatenation info and the
/// number of header octets (including the length octet). Unknown information elements are
/// skipped by their declared length, never interpreted.
fn parse_user_data_header(ud: &[u8]) -> Result<(Option<SmsMultipartInfo>, usize), SmsPduError> {
    let header_length = ud.first().copied().ok_or(SmsPduError::Multipart)?;
    let header_end = 1 + usize::from(header_length);
    let header = ud.get(1..header_end).ok_or(SmsPduError::Multipart)?;

    let mut multipart = None;
    let mut index = 0;
    while index < header.len() {
        let identifier = header[index];
        let length = usize::from(*header.get(index + 1).ok_or(SmsPduError::Multipart)?);
        let data = header
            .get(index + 2..index + 2 + length)
            .ok_or(SmsPduError::Multipart)?;
        match identifier {
            0x00 => {
                if data.len() != 3 {
                    return Err(SmsPduError::Multipart);
                }
                multipart = Some(validate_multipart(
                    SmsConcatReference::EightBit(data[0]),
                    data[1],
                    data[2],
                )?);
            }
            0x08 => {
                if data.len() != 4 {
                    return Err(SmsPduError::Multipart);
                }
                multipart = Some(validate_multipart(
                    SmsConcatReference::SixteenBit(u16::from_be_bytes([data[0], data[1]])),
                    data[2],
                    data[3],
                )?);
            }
            _ => {}
        }
        index += 2 + length;
    }
    Ok((multipart, header_end))
}

fn validate_multipart(
    reference: SmsConcatReference,
    total: u8,
    sequence: u8,
) -> Result<SmsMultipartInfo, SmsPduError> {
    if total == 0 || sequence == 0 || sequence > total {
        return Err(SmsPduError::Multipart);
    }
    Ok(SmsMultipartInfo {
        reference,
        total,
        sequence,
    })
}

/// Unpack the septet stream after any UDH (fill bits included) and decode the default alphabet
/// with the standard escape table. `udl` counts septets, header septets included.
fn decode_gsm7_user_data(
    ud: &[u8],
    udl: usize,
    has_user_data_header: bool,
) -> Result<(String, Option<SmsMultipartInfo>), SmsPduError> {
    let (multipart, first_septet) = if has_user_data_header {
        let (multipart, header_octets) = parse_user_data_header(ud)?;
        let first_septet = (header_octets * 8).div_ceil(7);
        (multipart, first_septet)
    } else {
        (None, 0)
    };
    let needed_octets = (udl * 7).div_ceil(8);
    if ud.len() < needed_octets {
        return Err(SmsPduError::Shape);
    }
    if first_septet > udl {
        return Err(SmsPduError::Multipart);
    }

    let mut body = String::new();
    let mut index = first_septet;
    while index < udl {
        let septet = gsm7_septet(ud, index);
        if septet == 0x1B {
            match index.checked_add(1).filter(|next| *next < udl) {
                Some(next) => {
                    body.push(gsm7_escape(gsm7_septet(ud, next)));
                    index = next;
                }
                None => body.push('\u{FFFD}'),
            }
        } else {
            body.push(gsm7_default(septet));
        }
        index += 1;
    }
    Ok((body, multipart))
}

fn decode_ucs2_user_data(
    ud: &[u8],
    udl: usize,
    has_user_data_header: bool,
) -> Result<(String, Option<SmsMultipartInfo>), SmsPduError> {
    let (multipart, body_start) = if has_user_data_header {
        let (multipart, header_octets) = parse_user_data_header(ud)?;
        (multipart, header_octets)
    } else {
        (None, 0)
    };
    if ud.len() < udl {
        return Err(SmsPduError::Shape);
    }
    if body_start > udl {
        return Err(SmsPduError::Multipart);
    }
    let body = &ud[body_start..udl];
    if body.len() % 2 != 0 {
        return Err(SmsPduError::Encoding);
    }
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16(&units).map_err(|_| SmsPduError::Encoding)?;
    Ok((text, multipart))
}

/// Extract one 7-bit septet from the LSB-first packed stream.
fn gsm7_septet(ud: &[u8], index: usize) -> u8 {
    let first_bit = index * 7;
    let mut value = 0_u8;
    for offset in 0..7 {
        let position = first_bit + offset;
        let bit = (ud[position / 8] >> (position % 8)) & 1;
        value |= bit << offset;
    }
    value
}

const GSM7_DEFAULT: [char; 128] = [
    '@', '£', '$', '¥', 'è', 'é', 'ù', 'ì', 'ò', 'Ç', '\n', 'Ø', 'ø', '\r', 'Å', 'å', 'Δ', '_',
    'Φ', 'Γ', 'Λ', 'Ω', 'Π', 'Ψ', 'Σ', 'Θ', 'Ξ', '\u{1B}', 'Æ', 'æ', 'ß', 'É', ' ', '!', '"', '#',
    '¤', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/', '0', '1', '2', '3', '4', '5', '6',
    '7', '8', '9', ':', ';', '<', '=', '>', '?', '¡', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'Ä', 'Ö',
    'Ñ', 'Ü', '§', '¿', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'ä', 'ö', 'ñ', 'ü', 'à',
];

fn gsm7_default(septet: u8) -> char {
    GSM7_DEFAULT
        .get(usize::from(septet))
        .copied()
        .unwrap_or('\u{FFFD}')
}

fn gsm7_escape(septet: u8) -> char {
    match septet {
        0x0A => '\u{000C}',
        0x14 => '^',
        0x28 => '{',
        0x29 => '}',
        0x2F => '\\',
        0x3C => '[',
        0x3D => '~',
        0x3E => ']',
        0x40 => '|',
        0x65 => '€',
        other => gsm7_default(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_sixteen_bit_references_with_equal_low_bytes_stay_distinct() {
        let first =
            decode_deliver_pdu("00440B912120550521F30000421020304050230A06080412340202EF35")
                .unwrap();
        let second =
            decode_deliver_pdu("00440B912120550521F30000421020304050230A06080456340202EF35")
                .unwrap();
        assert_ne!(first.multipart, second.multipart);
    }

    #[test]
    fn standalone_recipient_validation_matches_submit_and_never_normalizes() {
        for recipient in [
            "+12025550123",
            "+8613800138000",
            "+1",
            "+123456789012345",
            "10690000",
            "13800138000",
            "+0123",
            "+1234567890123456",
            " +123",
            "+123\rAT",
            "+１２３",
            "SERVICE",
            "",
        ] {
            assert_eq!(
                validate_sms_recipient(recipient).is_ok(),
                build_ucs2_submit(recipient, "test").is_ok()
            );
        }
    }

    // SMSC 00, DELIVER|MMS, sender +12025550123 (TOA 0x91), PID 00, DCS 00,
    // SCTS 24-01-02 03:04:05 +32, UDL 05, "hello" packed as septets.
    const GSM7_HELLO: &str = "00040B912120550521F300004210203040502305E8329BFD06";
    // Same transaction but with a real SMSC prefix: 06 91 44 77 55 66 11.
    const GSM7_HELLO_WITH_SMSC: &str =
        "06914477556611040B912120550521F300004210203040502305E8329BFD06";
    // SMSC 00, sender +8613800138000, DCS 08 (UCS-2), UDL 02, body U+4E2D.
    const UCS2_ZHONG: &str = "00040D91683108108300F0000842102030405023024E2D";
    // DELIVER|UDHI|MMS, UDH IEI 0x00 ref 5 total 2 seq 1, text "hi" starting at septet 7
    // (one fill bit after the six header octets).
    const GSM7_MULTIPART_8BIT_REF: &str =
        "00440B912120550521F300004210203040502309050003050201D069";
    // DELIVER|UDHI, UDH IEI 0x08 ref 0x1234 total 2 seq 2, text "ok" starting at septet 8.
    const GSM7_MULTIPART_16BIT_REF: &str =
        "00440B912120550521F30000421020304050230A06080412340202EF35";
    // DELIVER|UDHI, UDH IEI 0x00 ref 7 total 2 seq 1, UCS-2 body U+4E2D after the header.
    const UCS2_MULTIPART: &str = "00440B912120550521F3000842102030405023080500030702014E2D";

    #[test]
    fn build_ucs2_submit_matches_research_appendix_vector() {
        let pdu = build_ucs2_submit("+12025550123", "中").expect("appendix vector builds");
        assert_eq!(
            pdu.expose_for_confirmed_send(),
            "0001000B912120550521F30008024E2D"
        );
        assert_eq!(pdu.tpdu_octets, 15);
        assert!(!format!("{pdu:?}").contains("4E2D"));
        assert!(format!("{pdu:?}").contains("payload"));
    }

    #[test]
    fn build_ucs2_submit_accepts_the_exact_140_byte_limit() {
        let body = "中".repeat(70);
        let pdu = build_ucs2_submit("+12025550123", &body).expect("70 BMP chars fit");
        assert_eq!(pdu.tpdu_octets, 153);
        assert_eq!(pdu.expose_for_confirmed_send().len(), 308);
        assert!(
            pdu.expose_for_confirmed_send()
                .starts_with("0001000B912120550521F3")
        );
    }

    #[test]
    fn build_ucs2_submit_rejects_empty_body_emoji_and_oversize() {
        assert!(build_ucs2_submit("+12025550123", "").is_err());
        assert_eq!(
            build_ucs2_submit("+12025550123", "😀").unwrap_err(),
            SmsPduError::Encoding
        );
        assert_eq!(
            build_ucs2_submit("+12025550123", &"中".repeat(71)).unwrap_err(),
            SmsPduError::TooLong
        );
    }

    #[test]
    fn build_ucs2_submit_rejects_injection_and_malformed_recipients() {
        let invalid = [
            "12025550123",
            "+",
            "+012025550123",
            "+1202555012345678",
            "+12025550123\rAT",
            "+1202555012a",
            "+12 025",
        ];
        for recipient in invalid {
            assert_eq!(
                build_ucs2_submit(recipient, "中").unwrap_err(),
                SmsPduError::Number,
                "recipient: {recipient:?}"
            );
        }
    }

    #[test]
    fn decode_deliver_gsm7_single_message_with_scts() {
        let decoded = decode_deliver_pdu(GSM7_HELLO).expect("reference PDU decodes");
        assert_eq!(decoded.sender, "+12025550123");
        assert_eq!(decoded.timestamp.as_deref(), Some("240102030405+32"));
        assert_eq!(decoded.body, "hello");
        assert_eq!(decoded.encoding, SmsEncoding::Gsm7);
        assert_eq!(decoded.multipart, None);
        assert_eq!(decoded.read, None);
        assert!(!format!("{decoded:?}").contains("hello"));
        assert!(!format!("{decoded:?}").contains("12025550123"));
    }

    #[test]
    fn decode_deliver_ucs2_chinese_single_message() {
        let decoded = decode_deliver_pdu(UCS2_ZHONG).expect("UCS-2 PDU decodes");
        assert_eq!(decoded.sender, "+8613800138000");
        assert_eq!(decoded.body, "中");
        assert_eq!(decoded.encoding, SmsEncoding::Ucs2);
        assert_eq!(decoded.multipart, None);
    }

    #[test]
    fn decode_deliver_gsm7_multipart_uses_8bit_reference_and_fill_bits() {
        let decoded = decode_deliver_pdu(GSM7_MULTIPART_8BIT_REF).expect("multipart PDU decodes");
        assert_eq!(decoded.body, "hi");
        assert_eq!(decoded.encoding, SmsEncoding::Gsm7);
        assert_eq!(
            decoded.multipart,
            Some(SmsMultipartInfo {
                reference: SmsConcatReference::EightBit(5),
                total: 2,
                sequence: 1,
            })
        );
    }

    #[test]
    fn decode_deliver_gsm7_multipart_preserves_full_16bit_reference() {
        let decoded = decode_deliver_pdu(GSM7_MULTIPART_16BIT_REF).expect("16-bit ref decodes");
        assert_eq!(decoded.body, "ok");
        assert_eq!(
            decoded.multipart,
            Some(SmsMultipartInfo {
                reference: SmsConcatReference::SixteenBit(0x1234),
                total: 2,
                sequence: 2,
            })
        );
    }

    #[test]
    fn decode_deliver_ucs2_multipart_skips_the_header() {
        let decoded = decode_deliver_pdu(UCS2_MULTIPART).expect("UCS-2 multipart decodes");
        assert_eq!(decoded.body, "中");
        assert_eq!(decoded.encoding, SmsEncoding::Ucs2);
        assert_eq!(
            decoded.multipart,
            Some(SmsMultipartInfo {
                reference: SmsConcatReference::EightBit(7),
                total: 2,
                sequence: 1,
            })
        );
    }

    #[test]
    fn decode_deliver_ignores_a_nonempty_smsc() {
        let with_smsc = decode_deliver_pdu(GSM7_HELLO_WITH_SMSC).expect("SMSC prefix decodes");
        let without_smsc = decode_deliver_pdu(GSM7_HELLO).expect("empty SMSC decodes");
        assert_eq!(with_smsc.body, without_smsc.body);
        assert_eq!(with_smsc.sender, without_smsc.sender);
        assert_eq!(with_smsc.timestamp, without_smsc.timestamp);
    }

    #[test]
    fn decode_deliver_unknown_toa_has_no_plus_prefix() {
        let unknown = GSM7_HELLO.replacen("0B91", "0B81", 1);
        let decoded = decode_deliver_pdu(&unknown).expect("unknown TOA decodes");
        assert_eq!(decoded.sender, "12025550123");
    }

    #[test]
    fn decode_deliver_rejects_odd_or_invalid_ucs2() {
        let odd_udl = "00040D91683108108300F0000842102030405023034E2D00";
        assert_eq!(
            decode_deliver_pdu(odd_udl),
            Err(SmsPduError::Encoding),
            "odd UDL must not be guessed"
        );
        let lone_surrogate = "00040D91683108108300F000084210203040502302D800";
        assert_eq!(
            decode_deliver_pdu(lone_surrogate),
            Err(SmsPduError::Encoding),
            "illegal UTF-16 must not be guessed"
        );
    }

    #[test]
    fn decode_deliver_rejects_bad_hex_shapes_and_mti() {
        assert_eq!(decode_deliver_pdu(""), Err(SmsPduError::Shape));
        assert_eq!(decode_deliver_pdu("000"), Err(SmsPduError::Shape));
        assert_eq!(decode_deliver_pdu("00GG"), Err(SmsPduError::Shape));
        let submit_mti = "00010B912120550521F300004210203040502305E8329BFD06";
        assert_eq!(decode_deliver_pdu(submit_mti), Err(SmsPduError::Shape));
        let overlong = "00".repeat(MAX_PDU_HEX_CHARS / 2 + 1);
        assert_eq!(decode_deliver_pdu(&overlong), Err(SmsPduError::TooLong));
    }

    #[test]
    fn decode_deliver_handles_dcs_class_bits_and_unmodeled_dcs() {
        let gsm7_class1 = GSM7_HELLO.replacen("F3000042", "F3001142", 1);
        let decoded = decode_deliver_pdu(&gsm7_class1).expect("class-1 GSM7 decodes");
        assert_eq!(decoded.encoding, SmsEncoding::Gsm7);
        assert_eq!(decoded.body, "hello");

        let ucs2_class0 = UCS2_ZHONG.replacen("F0000842", "F0001842", 1);
        let decoded = decode_deliver_pdu(&ucs2_class0).expect("class-0 UCS2 decodes");
        assert_eq!(decoded.encoding, SmsEncoding::Ucs2);
        assert_eq!(decoded.body, "中");

        let eight_bit = GSM7_HELLO.replacen("F3000042", "F3000442", 1);
        let decoded = decode_deliver_pdu(&eight_bit).expect("unmodeled DCS decodes as Other");
        assert_eq!(decoded.encoding, SmsEncoding::Other);
        assert_eq!(decoded.body, "");
        assert_eq!(decoded.multipart, None);
    }

    #[test]
    fn decode_deliver_rejects_truncated_ud_and_inconsistent_multipart() {
        let truncated = "00040B912120550521F300004210203040502305E8329B";
        assert_eq!(decode_deliver_pdu(truncated), Err(SmsPduError::Shape));

        let bad_sequence = "00440B912120550521F300004210203040502309050003050203D069";
        assert_eq!(
            decode_deliver_pdu(bad_sequence),
            Err(SmsPduError::Multipart)
        );
    }
}
