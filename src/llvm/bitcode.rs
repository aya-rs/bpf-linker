use std::collections::HashMap;

#[expect(missing_copy_implementations, reason = "not needed")]
#[derive(Debug, thiserror::Error)]
pub enum BitcodeError {
    #[error("bitcode has invalid size, expected at least 8 bytes, got {0}")]
    InvalidSize(usize),
    #[error("bitcode is not 32-bit aligned")]
    Misaligned,
    #[error("missing bitcode magic header")]
    MissingMagicHeader,
    #[error("bitcode cursor seek out of bounds")]
    CursorOutOfBounds,
    #[error("unexpected end of bitcode")]
    UnexpectedEnd,
    #[error("unsupported abbreviation encoding: {0}")]
    UnsupportedAbbreviationEncoding(usize),
    #[error("unsupported abbreviated record ID: {0}")]
    UnsupportedAbbreviatedRecordID(usize),
    #[error("abbreviation {0} referenced before definition")]
    UnknownAbbreviation(usize),
    #[error("array abbreviation missing element encoding")]
    MissingArrayElementEncoding,
    #[error("array element encoding must be non-literal")]
    InvalidArrayElementEncoding,
    #[error("abbreviated record missing leading code")]
    MissingRecordCode,
    #[error("invalid CHAR6 value {0}")]
    InvalidChar6(u64),
    #[error("BLOCKINFO block missing target id before defining abbreviation")]
    BlockInfoMissingTarget,
    #[error("BLOCKINFO SETBID record missing operand")]
    BlockInfoMissingSetBidOperand,
    #[error("BLOCKINFO SETBID value {0} does not fit in u32")]
    BlockInfoInvalidSetBid(u64),
    #[error("mising identification string")]
    MissingIdentificationString,
    #[error("value {0} exceeds supported range for u32")]
    ValueOutOfRangeU32(u64),
    #[error("value {0} exceeds supported range for usize")]
    ValueOutOfRangeUsize(u64),
    #[error("identification string contains invalid byte value {0}")]
    InvalidIdentificationByte(u64),
}

pub(crate) fn identification_string(buffer: &[u8]) -> Result<String, BitcodeError> {
    if buffer.len() < 8 {
        return Err(BitcodeError::InvalidSize(buffer.len()));
    }
    if !buffer.len().is_multiple_of(4) {
        return Err(BitcodeError::Misaligned);
    }

    let mut words = Vec::with_capacity(buffer.len() / 4);
    for chunk in buffer.chunks_exact(4) {
        words.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }

    const BITCODE_MAGIC: u32 = 0xdec0_4342;
    if words.first().copied() != Some(BITCODE_MAGIC) {
        return Err(BitcodeError::MissingMagicHeader);
    }

    let mut cursor = BitCursor::new(&words);
    cursor.seek_to_bit(32)?;

    let mut block_infos: HashMap<u32, Vec<Abbrev>> = HashMap::new();
    let mut current_blockinfo_target = None;
    let mut blocks = vec![BlockState::root()];

    while blocks.last().is_some() {
        if cursor.is_eof() {
            break;
        }

        let (code_size, block_id) = {
            let state = blocks.last().expect("block stack not empty");
            (state.code_size, state.block_id)
        };
        let abbrev_id = cursor.read_bits(code_size)?;
        match abbrev_id {
            ABBREV_ID_END_BLOCK => {
                cursor.align32()?;
                if let Some(popped) = blocks.pop()
                    && popped.block_id == Some(BLOCKINFO_BLOCK_ID)
                {
                    current_blockinfo_target = None;
                }
                if blocks.is_empty() {
                    break;
                }
            }
            ABBREV_ID_ENTER_SUBBLOCK => {
                let block_id = cursor.read_vbr_u32(SUBBLOCK_ID_VBR_WIDTH)?;
                let new_code_size = cursor.read_vbr_usize(SUBBLOCK_CODE_SIZE_VBR_WIDTH)?;
                cursor.align32()?;
                let _len_in_words = cursor.read_bits(32)?;
                let inherited = block_infos.get(&block_id).cloned().unwrap_or_default();
                blocks.push(BlockState::new(block_id, new_code_size, inherited));
                if block_id == BLOCKINFO_BLOCK_ID {
                    current_blockinfo_target = None;
                }
            }
            ABBREV_ID_DEFINE_ABBREV => {
                let abbrev = read_abbrev(&mut cursor)?;
                if block_id == Some(BLOCKINFO_BLOCK_ID) {
                    let target =
                        current_blockinfo_target.ok_or(BitcodeError::BlockInfoMissingTarget)?;
                    block_infos.entry(target).or_default().push(abbrev);
                } else {
                    let state = blocks.last_mut().expect("block stack not empty");
                    state.abbrevs.push(abbrev);
                }
            }
            ABBREV_ID_UNABBREV_RECORD => {
                let record = read_unabbrev_record(&mut cursor)?;
                if let Some(string) =
                    process_record(record, block_id, &mut current_blockinfo_target)?
                {
                    return Ok(string);
                }
            }
            other => {
                if other < ABBREV_ID_UNABBREV_RECORD + 1 {
                    return Err(BitcodeError::UnsupportedAbbreviatedRecordID(other));
                }
                let abbrev_index = other - (ABBREV_ID_UNABBREV_RECORD + 1);
                let state = blocks.last_mut().expect("block stack not empty");
                let abbrev = state
                    .abbrevs
                    .get(abbrev_index)
                    .ok_or(BitcodeError::UnknownAbbreviation(other))?;
                let record = read_abbrev_record(&mut cursor, abbrev)?;
                if let Some(string) =
                    process_record(record, block_id, &mut current_blockinfo_target)?
                {
                    return Ok(string);
                }
            }
        }
    }

    Err(BitcodeError::MissingIdentificationString)
}

const ABBREV_ID_END_BLOCK: usize = 0;
const ABBREV_ID_ENTER_SUBBLOCK: usize = 1;
const ABBREV_ID_DEFINE_ABBREV: usize = 2;
const ABBREV_ID_UNABBREV_RECORD: usize = 3;

const BLOCKINFO_BLOCK_ID: u32 = 0;
const BLOCKINFO_CODE_SETBID: u32 = 1;
const IDENTIFICATION_BLOCK_ID: u32 = 13;
const IDENTIFICATION_CODE_STRING: u32 = 1;

/// VBR width used when decoding block IDs inside `ENTER_SUBBLOCK` records.
const SUBBLOCK_ID_VBR_WIDTH: usize = 8;
/// VBR width that encodes a subblock's local abbreviation bit width.
const SUBBLOCK_CODE_SIZE_VBR_WIDTH: usize = 4;
/// VBR width for unabbreviated record codes.
const RECORD_CODE_VBR_WIDTH: usize = 6;
/// VBR width for the number of operands in unabbreviated records.
const RECORD_NUM_OPERANDS_VBR_WIDTH: usize = 6;
/// VBR width for each operand within an unabbreviated record.
const RECORD_OPERAND_VBR_WIDTH: usize = 6;
/// VBR width that encodes how many ops a `DEFINE_ABBREV` entry has.
const ABBREV_NUM_OPERANDS_VBR_WIDTH: usize = 5;
/// VBR width for literal values inside `DEFINE_ABBREV`.
const LITERAL_VBR_WIDTH: usize = 8;
/// VBR width for data attached to certain abbrev encodings (`Array`/`Char6`).
const ABBREV_ENCODING_DATA_VBR_WIDTH: usize = 5;
/// VBR width used for array/blob lengths in abbreviated records.
const LENGTH_VBR_WIDTH: usize = 6;

struct BlockState {
    block_id: Option<u32>,
    code_size: usize,
    abbrevs: Vec<Abbrev>,
}

impl BlockState {
    fn root() -> Self {
        Self {
            block_id: None,
            code_size: 2,
            abbrevs: Vec::new(),
        }
    }

    fn new(block_id: u32, code_size: usize, abbrevs: Vec<Abbrev>) -> Self {
        Self {
            block_id: Some(block_id),
            code_size,
            abbrevs,
        }
    }
}

#[derive(Clone)]
struct Abbrev {
    ops: Vec<AbbrevOp>,
}

#[derive(Clone)]
enum AbbrevOp {
    Literal(u64),
    Encoding(AbbrevEncoding),
}

#[derive(Clone)]
enum AbbrevEncoding {
    Fixed(usize),
    Vbr(usize),
    Char6,
    Array(Box<AbbrevEncoding>),
    Blob,
}

/// Bit-level reader over 32-bit word slices.
/// Tracks the current bit offset and supports arbitrary-width bitcode fields.
struct BitCursor<'a> {
    words: &'a [u32],
    bit_len: usize,
    bit_pos: usize,
}

impl<'a> BitCursor<'a> {
    fn new(words: &'a [u32]) -> Self {
        Self {
            words,
            bit_len: words.len() * 32,
            bit_pos: 0,
        }
    }

    fn seek_to_bit(&mut self, bit: usize) -> Result<(), BitcodeError> {
        if bit > self.bit_len {
            return Err(BitcodeError::CursorOutOfBounds);
        }
        self.bit_pos = bit;
        Ok(())
    }

    fn is_eof(&self) -> bool {
        self.bit_pos >= self.bit_len
    }

    /// Reads `n` bits from the current position, stitching across word
    /// boundaries when needed, and advances the cursor by that many bits.
    fn read_bits(&mut self, n: usize) -> Result<usize, BitcodeError> {
        if n == 0 {
            return Ok(0);
        }
        if self.bit_pos + n > self.bit_len {
            return Err(BitcodeError::UnexpectedEnd);
        }

        let mut result = 0usize;
        let mut read = 0;

        while read < n {
            let word_index = self.bit_pos >> 5;
            let bit_index = self.bit_pos & 31;
            let bits_available = 32 - bit_index;
            let take = std::cmp::min(bits_available, n - read);
            let mask = if take == 32 {
                usize::MAX
            } else {
                (1usize << take) - 1
            };
            let chunk = ((self.words[word_index] as usize) >> bit_index) & mask;
            result |= chunk << read;
            self.bit_pos += take;
            read += take;
        }

        Ok(result)
    }

    /// Reads an LLVM variable-bit-rate (VBR) integer.
    /// Each `width`-bit chunk uses the MSB as a continuation flag, with the
    /// remaining bits appended LSB-first until a chunk clears the flag.
    fn read_vbr(&mut self, width: usize) -> Result<u64, BitcodeError> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            let piece = self.read_bits(width)? as u64;
            let continue_bit = 1u64 << (width - 1);
            let value = piece & (continue_bit - 1);
            result |= value << shift;
            if piece & continue_bit == 0 {
                break;
            }
            shift += width - 1;
        }
        Ok(result)
    }

    fn read_vbr_u32(&mut self, width: usize) -> Result<u32, BitcodeError> {
        let value = self.read_vbr(width)?;
        value
            .try_into()
            .map_err(|_| BitcodeError::ValueOutOfRangeU32(value))
    }

    fn read_vbr_usize(&mut self, width: usize) -> Result<usize, BitcodeError> {
        let value = self.read_vbr(width)?;
        value
            .try_into()
            .map_err(|_| BitcodeError::ValueOutOfRangeUsize(value))
    }

    /// Skips padding so the cursor advances to the next 32-bit boundary.
    /// LLVM blocks require subsequent contents to start on word-aligned offsets.
    fn align32(&mut self) -> Result<(), BitcodeError> {
        let remainder = self.bit_pos & 31;
        if remainder != 0 {
            let to_skip = 32 - remainder;
            let _ = self.read_bits(to_skip)?;
        }
        Ok(())
    }
}

/// Unabbreviated LLVM.ident record containing the opcode and raw operand payload.
struct Record {
    code: u32,
    operands: Vec<u64>,
}

fn read_unabbrev_record(cursor: &mut BitCursor<'_>) -> Result<Record, BitcodeError> {
    let code = cursor.read_vbr_u32(RECORD_CODE_VBR_WIDTH)?;
    let num_ops = cursor.read_vbr_usize(RECORD_NUM_OPERANDS_VBR_WIDTH)?;
    let mut operands = Vec::with_capacity(num_ops);
    for _ in 0..num_ops {
        operands.push(cursor.read_vbr(RECORD_OPERAND_VBR_WIDTH)?);
    }
    Ok(Record { code, operands })
}

fn process_record(
    record: Record,
    block_id: Option<u32>,
    current_blockinfo_target: &mut Option<u32>,
) -> Result<Option<String>, BitcodeError> {
    if block_id == Some(BLOCKINFO_BLOCK_ID) {
        handle_blockinfo_record(&record, current_blockinfo_target)?;
        return Ok(None);
    }
    if block_id == Some(IDENTIFICATION_BLOCK_ID) && record.code == IDENTIFICATION_CODE_STRING {
        let string = record_to_ident_string(record)?;
        return Ok(Some(string));
    }
    Ok(None)
}

fn record_to_ident_string(record: Record) -> Result<String, BitcodeError> {
    let bytes = record
        .operands
        .into_iter()
        .map(|op| u8::try_from(op).map_err(|_| BitcodeError::InvalidIdentificationByte(op)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_abbrev(cursor: &mut BitCursor<'_>) -> Result<Abbrev, BitcodeError> {
    let mut remaining = cursor.read_vbr_usize(ABBREV_NUM_OPERANDS_VBR_WIDTH)?;
    let mut ops = Vec::with_capacity(remaining);
    while remaining > 0 {
        ops.push(read_abbrev_op(cursor, &mut remaining)?);
    }
    Ok(Abbrev { ops })
}

fn read_abbrev_op(
    cursor: &mut BitCursor<'_>,
    remaining: &mut usize,
) -> Result<AbbrevOp, BitcodeError> {
    *remaining -= 1;
    let is_literal = cursor.read_bits(1)? != 0;
    if is_literal {
        let literal = cursor.read_vbr(LITERAL_VBR_WIDTH)?;
        Ok(AbbrevOp::Literal(literal))
    } else {
        let encoding = read_abbrev_encoding(cursor, remaining)?;
        Ok(AbbrevOp::Encoding(encoding))
    }
}

fn read_abbrev_encoding(
    cursor: &mut BitCursor<'_>,
    remaining: &mut usize,
) -> Result<AbbrevEncoding, BitcodeError> {
    let encoding_kind = cursor.read_bits(3)?;
    match encoding_kind {
        1 => {
            let width = cursor.read_vbr_usize(ABBREV_ENCODING_DATA_VBR_WIDTH)?;
            Ok(AbbrevEncoding::Fixed(width))
        }
        2 => {
            let width = cursor.read_vbr_usize(ABBREV_ENCODING_DATA_VBR_WIDTH)?;
            Ok(AbbrevEncoding::Vbr(width))
        }
        3 => {
            if *remaining == 0 {
                return Err(BitcodeError::MissingArrayElementEncoding);
            }
            let element = read_abbrev_op(cursor, remaining)?;
            match element {
                AbbrevOp::Literal(_) => Err(BitcodeError::InvalidArrayElementEncoding),
                AbbrevOp::Encoding(enc) => Ok(AbbrevEncoding::Array(Box::new(enc))),
            }
        }
        4 => Ok(AbbrevEncoding::Char6),
        5 => Ok(AbbrevEncoding::Blob),
        other => Err(BitcodeError::UnsupportedAbbreviationEncoding(other)),
    }
}

fn read_abbrev_record(cursor: &mut BitCursor<'_>, abbrev: &Abbrev) -> Result<Record, BitcodeError> {
    let mut code = None;
    let mut operands = Vec::new();
    for op in &abbrev.ops {
        match op {
            AbbrevOp::Literal(value) => push_value(*value, &mut code, &mut operands)?,
            AbbrevOp::Encoding(encoding) => {
                read_encoded_values(cursor, encoding, &mut code, &mut operands)?
            }
        }
    }
    let code = code.ok_or(BitcodeError::MissingRecordCode)?;
    Ok(Record { code, operands })
}

fn read_encoded_values(
    cursor: &mut BitCursor<'_>,
    encoding: &AbbrevEncoding,
    code: &mut Option<u32>,
    operands: &mut Vec<u64>,
) -> Result<(), BitcodeError> {
    match encoding {
        AbbrevEncoding::Fixed(width) => {
            let value = cursor.read_bits(*width)? as u64;
            push_value(value, code, operands)?;
        }
        AbbrevEncoding::Vbr(width) => {
            let value = cursor.read_vbr(*width)?;
            push_value(value, code, operands)?;
        }
        AbbrevEncoding::Char6 => {
            let raw = cursor.read_bits(6)? as u64;
            let ch = decode_char6(raw)?;
            push_value(u64::from(ch), code, operands)?;
        }
        AbbrevEncoding::Array(element) => {
            let len = cursor.read_vbr_usize(LENGTH_VBR_WIDTH)?;
            for _ in 0..len {
                read_encoded_values(cursor, element, code, operands)?;
            }
        }
        AbbrevEncoding::Blob => {
            let len = cursor.read_vbr_usize(LENGTH_VBR_WIDTH)?;
            cursor.align32()?;
            for _ in 0..len {
                let byte = cursor.read_bits(8)? as u64;
                push_value(byte, code, operands)?;
            }
            cursor.align32()?;
        }
    }
    Ok(())
}

fn push_value(
    value: u64,
    code: &mut Option<u32>,
    operands: &mut Vec<u64>,
) -> Result<(), BitcodeError> {
    if code.is_some() {
        operands.push(value);
    } else {
        let record_code =
            u32::try_from(value).map_err(|_| BitcodeError::ValueOutOfRangeU32(value))?;
        *code = Some(record_code);
    }
    Ok(())
}

fn decode_char6(value: u64) -> Result<u8, BitcodeError> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "values are checked before casting"
    )]
    let ch = match value {
        0..=25 => b'a' + value as u8,
        26..=51 => b'A' + (value as u8 - 26),
        52..=61 => b'0' + (value as u8 - 52),
        62 => b'.',
        63 => b'_',
        _ => return Err(BitcodeError::InvalidChar6(value)),
    };
    Ok(ch)
}

fn handle_blockinfo_record(record: &Record, current: &mut Option<u32>) -> Result<(), BitcodeError> {
    if record.code == BLOCKINFO_CODE_SETBID {
        let value = record
            .operands
            .first()
            .copied()
            .ok_or(BitcodeError::BlockInfoMissingSetBidOperand)?;
        let block_id =
            u32::try_from(value).map_err(|_| BitcodeError::BlockInfoInvalidSetBid(value))?;
        *current = Some(block_id);
    }
    Ok(())
}
