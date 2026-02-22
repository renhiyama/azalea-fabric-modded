//! Packet decoders for modded networking protocols.
//!
//! This module provides decoders for common modded Minecraft packet formats
//! that the bot receives but doesn't fully implement.

use std::io::Cursor;

// ---------------------------------------------------------------------------
// VarInt helpers
// ---------------------------------------------------------------------------

/// Reads a Minecraft-style VarInt from a cursor.
pub fn read_varint(cursor: &mut Cursor<&[u8]>) -> Option<i32> {
    let mut result = 0i32;
    let mut shift = 0u32;
    loop {
        if shift >= 35 {
            return None; // overflow
        }
        let mut byte = [0u8; 1];
        use std::io::Read;
        cursor.read_exact(&mut byte).ok()?;
        let b = byte[0];
        result |= ((b & 0x7F) as i32) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            return Some(result);
        }
    }
}

/// Reads a Minecraft-style VarInt as u32.
pub fn read_varint_u32(cursor: &mut Cursor<&[u8]>) -> Option<u32> {
    read_varint(cursor).map(|v| v as u32)
}

/// Reads a Minecraft-style prefixed UTF-8 string (VarInt length + bytes).
pub fn read_string(cursor: &mut Cursor<&[u8]>) -> Option<String> {
    // Try VarInt first (Minecraft standard)
    let len = read_varint(cursor)? as usize;
    let mut bytes = vec![0u8; len];
    use std::io::Read;
    cursor.read_exact(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// Reads a simple byte-prefixed string (single byte length + UTF-8).
pub fn read_string_byte_prefix(cursor: &mut Cursor<&[u8]>) -> Option<String> {
    let mut len_byte = [0u8; 1];
    use std::io::Read;
    cursor.read_exact(&mut len_byte).ok()?;
    let len = len_byte[0] as usize;
    let mut bytes = vec![0u8; len];
    cursor.read_exact(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

// ---------------------------------------------------------------------------
// CCA Entity Sync Packet Decoder
// ---------------------------------------------------------------------------

/// Decoded Cardinal Components API entity sync packet.
#[derive(Debug, Clone)]
pub struct CcaEntitySyncPacket {
    /// Entity ID being synced.
    pub entity_id: i32,
    /// Components attached to the entity.
    pub components: Vec<CcaComponent>,
}

/// A single component from a CCA sync packet.
#[derive(Debug, Clone)]
pub struct CcaComponent {
    /// Component type identifier (e.g. "travelersbackpack:backpack").
    pub component_type: String,
    /// Component data (usually NBT).
    pub data: ComponentData,
}

/// Parsed component data.
#[derive(Debug, Clone)]
pub enum ComponentData {
    /// Raw NBT data (not parsed).
    Nbt(Vec<u8>),
    /// Successfully parsed NBT.
    ParsedNbt(NbtCompound),
    /// Unknown format.
    Unknown(Vec<u8>),
}

/// NBT Compound tag (simplified representation).
#[derive(Debug, Clone, Default)]
pub struct NbtCompound {
    /// Tag name-value pairs.
    pub tags: Vec<(String, NbtTag)>,
}

/// NBT tag value (simplified).
#[derive(Debug, Clone)]
pub enum NbtTag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<NbtTag>),
    Compound(NbtCompound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// Decodes a CCA entity_sync packet payload.
///
/// Packet format (as of CCA 6.x):
/// - VarInt: entity ID
/// - For each component:
///   - String: component type identifier
///   - NBT: component data
pub fn decode_cca_entity_sync(data: &[u8]) -> Result<CcaEntitySyncPacket, String> {
    let mut cursor = Cursor::new(data);

    // Read entity ID
    let entity_id =
        read_varint(&mut cursor).ok_or_else(|| "Failed to read entity ID".to_string())?;

    let mut components = Vec::new();

    // Read components until we run out of data
    while (cursor.position() as usize) < data.len() {
        // Read component type identifier
        let component_type =
            read_string(&mut cursor).ok_or_else(|| "Failed to read component type".to_string())?;

        // Read NBT data (remaining bytes for this component)
        // CCA sends NBT as: short length + NBT payload
        let nbt_start = cursor.position() as usize;
        let remaining = data.len() - nbt_start;

        // Try to read NBT length (short, big-endian)
        if remaining >= 2 {
            let nbt_len = u16::from_be_bytes([data[nbt_start], data[nbt_start + 1]]) as usize;
            if remaining >= nbt_len + 2 {
                let nbt_data = data[nbt_start + 2..nbt_start + 2 + nbt_len].to_vec();
                cursor.set_position((nbt_start + 2 + nbt_len) as u64);

                // Try to parse NBT
                let component_data = match parse_nbt(&nbt_data) {
                    Ok(nbt) => ComponentData::ParsedNbt(nbt),
                    Err(_) => ComponentData::Nbt(nbt_data),
                };

                components.push(CcaComponent {
                    component_type,
                    data: component_data,
                });
                continue;
            }
        }

        // If we can't parse NBT structure, just take remaining data
        let remaining_data = data[cursor.position() as usize..].to_vec();
        components.push(CcaComponent {
            component_type,
            data: ComponentData::Unknown(remaining_data),
        });
        break;
    }

    Ok(CcaEntitySyncPacket {
        entity_id,
        components,
    })
}

/// Parses NBT data into a structured format.
///
/// NBT format:
/// - Tag type (1 byte)
/// - Tag name (short length + UTF-8)
/// - Tag payload (depends on type)
/// - End tag (0x00) for compounds
pub fn parse_nbt(data: &[u8]) -> Result<NbtCompound, String> {
    if data.is_empty() {
        return Ok(NbtCompound::default());
    }

    let mut cursor = Cursor::new(data);
    read_nbt_compound(&mut cursor)
}

fn read_nbt_compound(cursor: &mut Cursor<&[u8]>) -> Result<NbtCompound, String> {
    use std::io::Read;

    let mut compound = NbtCompound::default();

    loop {
        // Read tag type
        let mut type_byte = [0u8; 1];
        cursor
            .read_exact(&mut type_byte)
            .map_err(|e| e.to_string())?;
        let tag_type = type_byte[0];

        // End tag
        if tag_type == 0 {
            break;
        }

        // Read tag name
        let tag_name = read_nbt_string(cursor)?;

        // Read tag value based on type
        let tag_value = read_nbt_payload(cursor, tag_type)?;

        compound.tags.push((tag_name, tag_value));
    }

    Ok(compound)
}

fn read_nbt_string(cursor: &mut Cursor<&[u8]>) -> Result<String, String> {
    use std::io::Read;

    let mut len_bytes = [0u8; 2];
    cursor
        .read_exact(&mut len_bytes)
        .map_err(|e| e.to_string())?;
    let len = u16::from_be_bytes(len_bytes) as usize;

    let mut bytes = vec![0u8; len];
    cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;

    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn read_nbt_payload(cursor: &mut Cursor<&[u8]>, tag_type: u8) -> Result<NbtTag, String> {
    use std::io::Read;

    match tag_type {
        1 => {
            // TAG_Byte
            let mut byte = [0u8; 1];
            cursor.read_exact(&mut byte).map_err(|e| e.to_string())?;
            Ok(NbtTag::Byte(byte[0] as i8))
        }
        2 => {
            // TAG_Short
            let mut bytes = [0u8; 2];
            cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(NbtTag::Short(i16::from_be_bytes(bytes)))
        }
        3 => {
            // TAG_Int
            let mut bytes = [0u8; 4];
            cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(NbtTag::Int(i32::from_be_bytes(bytes)))
        }
        4 => {
            // TAG_Long
            let mut bytes = [0u8; 8];
            cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(NbtTag::Long(i64::from_be_bytes(bytes)))
        }
        5 => {
            // TAG_Float
            let mut bytes = [0u8; 4];
            cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(NbtTag::Float(f32::from_be_bytes(bytes)))
        }
        6 => {
            // TAG_Double
            let mut bytes = [0u8; 8];
            cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(NbtTag::Double(f64::from_be_bytes(bytes)))
        }
        7 => {
            // TAG_Byte_Array
            let mut len_bytes = [0u8; 4];
            cursor
                .read_exact(&mut len_bytes)
                .map_err(|e| e.to_string())?;
            let len = i32::from_be_bytes(len_bytes) as usize;
            let mut bytes = vec![0u8; len];
            cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            Ok(NbtTag::ByteArray(
                bytes.into_iter().map(|b| b as i8).collect(),
            ))
        }
        8 => {
            // TAG_String
            Ok(NbtTag::String(read_nbt_string(cursor)?))
        }
        9 => {
            // TAG_List
            let mut type_byte = [0u8; 1];
            cursor
                .read_exact(&mut type_byte)
                .map_err(|e| e.to_string())?;
            let list_type = type_byte[0];

            let mut len_bytes = [0u8; 4];
            cursor
                .read_exact(&mut len_bytes)
                .map_err(|e| e.to_string())?;
            let len = i32::from_be_bytes(len_bytes) as usize;

            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(read_nbt_payload(cursor, list_type)?);
            }
            Ok(NbtTag::List(items))
        }
        10 => {
            // TAG_Compound
            Ok(NbtTag::Compound(read_nbt_compound(cursor)?))
        }
        11 => {
            // TAG_Int_Array
            let mut len_bytes = [0u8; 4];
            cursor
                .read_exact(&mut len_bytes)
                .map_err(|e| e.to_string())?;
            let len = i32::from_be_bytes(len_bytes) as usize;

            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                let mut bytes = [0u8; 4];
                cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
                items.push(i32::from_be_bytes(bytes));
            }
            Ok(NbtTag::IntArray(items))
        }
        12 => {
            // TAG_Long_Array
            let mut len_bytes = [0u8; 4];
            cursor
                .read_exact(&mut len_bytes)
                .map_err(|e| e.to_string())?;
            let len = i32::from_be_bytes(len_bytes) as usize;

            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                let mut bytes = [0u8; 8];
                cursor.read_exact(&mut bytes).map_err(|e| e.to_string())?;
                items.push(i64::from_be_bytes(bytes));
            }
            Ok(NbtTag::LongArray(items))
        }
        _ => Err(format!("Unknown NBT tag type: {}", tag_type)),
    }
}

// ---------------------------------------------------------------------------
// Pretty printing
// ---------------------------------------------------------------------------

impl std::fmt::Display for CcaEntitySyncPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "CCA Entity Sync (entity_id={})", self.entity_id)?;
        for component in &self.components {
            writeln!(f, "  Component: {}", component.component_type)?;
            match &component.data {
                ComponentData::ParsedNbt(nbt) => {
                    for (name, value) in &nbt.tags {
                        writeln!(f, "    {}: {}", name, format_nbt_tag(value))?;
                    }
                }
                ComponentData::Nbt(data) => {
                    writeln!(f, "    NBT data: {} bytes", data.len())?;
                }
                ComponentData::Unknown(data) => {
                    writeln!(f, "    Unknown: {} bytes", data.len())?;
                }
            }
        }
        Ok(())
    }
}

/// Pretty-prints an NBT tag value.
pub fn format_nbt_tag(tag: &NbtTag) -> String {
    match tag {
        NbtTag::Byte(v) => format!("{}b", v),
        NbtTag::Short(v) => format!("{}s", v),
        NbtTag::Int(v) => format!("{}", v),
        NbtTag::Long(v) => format!("{}L", v),
        NbtTag::Float(v) => format!("{}f", v),
        NbtTag::Double(v) => format!("{}", v),
        NbtTag::ByteArray(v) => format!("[{} bytes]", v.len()),
        NbtTag::String(v) => format!("\"{}\"", v),
        NbtTag::List(v) => format!("[{} items]", v.len()),
        NbtTag::Compound(c) => format!("{{{} tags}}", c.tags.len()),
        NbtTag::IntArray(v) => format!("[{} ints]", v.len()),
        NbtTag::LongArray(v) => format!("[{} longs]", v.len()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint() {
        let data = vec![0x01];
        let mut cursor = Cursor::new(data.as_slice());
        assert_eq!(read_varint(&mut cursor), Some(1));

        let data = vec![0x7F];
        let mut cursor = Cursor::new(data.as_slice());
        assert_eq!(read_varint(&mut cursor), Some(127));

        let data = vec![0x80, 0x01];
        let mut cursor = Cursor::new(data.as_slice());
        assert_eq!(read_varint(&mut cursor), Some(128));
    }
}
