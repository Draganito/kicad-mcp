//! Minimal protobuf wire helpers.
//!
//! Used to set `Pad.net` (field 4) without decoding the rest of a pad —
//! a partial prost round-trip would drop padstack geometry.

/// Replace or insert a length-delimited field (`wire_type = 2`).
pub fn set_len_field(buf: &[u8], field: u32, payload: &[u8]) -> Result<Vec<u8>, String> {
    let tag = (field << 3) | 2;
    let mut out = Vec::with_capacity(buf.len() + payload.len() + 8);
    let mut i = 0;
    while i < buf.len() {
        let (key, n) = read_varint(&buf[i..])?;
        i += n;
        let fld = (key >> 3) as u32;
        let wt = (key & 7) as u8;
        if fld == field {
            skip_value(buf, &mut i, wt)?;
            continue;
        }
        write_varint(&mut out, key);
        copy_value(buf, &mut i, wt, &mut out)?;
    }
    write_varint(&mut out, tag as u64);
    write_varint(&mut out, payload.len() as u64);
    out.extend_from_slice(payload);
    Ok(out)
}

/// `kiapi.board.types.Net` with `code` (field 1 / NetCode.value) and `name`.
/// KiCad 9.0.2 still keys pads by net code; name-only encodes as code 0 / unconnected.
pub fn encode_net(name: &str, code: i32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(name.len() + 16);
    let mut code_msg = Vec::new();
    code_msg.push((1 << 3) | 0);
    write_varint(&mut code_msg, code as u64);
    buf.push((1 << 3) | 2);
    write_varint(&mut buf, code_msg.len() as u64);
    buf.extend_from_slice(&code_msg);
    buf.push((2 << 3) | 2);
    write_varint(&mut buf, name.len() as u64);
    buf.extend_from_slice(name.as_bytes());
    buf
}

/// Name-only (KiCad 10). Prefer [`encode_net`] on KiCad 9.
pub fn encode_net_name(name: &str) -> Vec<u8> {
    encode_net(name, 0)
}

/// KiCad's unconnected sentinel: net code 0, name `"unconnected"`.
pub fn encode_unconnected() -> Vec<u8> {
    encode_net("unconnected", 0)
}

/// Map every occurrence of a length-delimited field (`wire_type = 2`).
/// Other fields are copied unchanged. Used to splice nested pad Anys
/// inside a `FootprintInstance` without a prost round-trip of the parent.
pub fn map_len_fields(
    buf: &[u8],
    field: u32,
    mut f: impl FnMut(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(buf.len());
    let mut i = 0;
    let mut seen = false;
    while i < buf.len() {
        let (key, n) = read_varint(&buf[i..])?;
        i += n;
        let fld = (key >> 3) as u32;
        let wt = (key & 7) as u8;
        if fld == field {
            if wt != 2 {
                return Err(format!(
                    "protobuf field {field} is wire type {wt}, expected length-delimited"
                ));
            }
            seen = true;
            let payload = take_len_payload(buf, &mut i)?;
            let mapped = f(payload)?;
            write_varint(&mut out, key);
            write_varint(&mut out, mapped.len() as u64);
            out.extend_from_slice(&mapped);
            continue;
        }
        write_varint(&mut out, key);
        copy_value(buf, &mut i, wt, &mut out)?;
    }
    if !seen {
        return Ok(buf.to_vec());
    }
    Ok(out)
}

fn take_len_payload<'a>(buf: &'a [u8], i: &mut usize) -> Result<&'a [u8], String> {
    let (len, n) = read_varint(&buf[*i..])?;
    *i += n;
    let len = len as usize;
    let start = *i;
    *i = i
        .checked_add(len)
        .ok_or("truncated length-delimited field")?;
    if *i > buf.len() {
        return Err("truncated length-delimited field".into());
    }
    Ok(&buf[start..*i])
}

fn copy_value(buf: &[u8], i: &mut usize, wt: u8, out: &mut Vec<u8>) -> Result<(), String> {
    let start = *i;
    skip_value(buf, i, wt)?;
    out.extend_from_slice(&buf[start..*i]);
    Ok(())
}

fn skip_value(buf: &[u8], i: &mut usize, wt: u8) -> Result<(), String> {
    match wt {
        0 => {
            let (_, n) = read_varint(&buf[*i..])?;
            *i += n;
        }
        1 => {
            *i = i.checked_add(8).ok_or("truncated 64-bit protobuf field")?;
            if *i > buf.len() {
                return Err("truncated 64-bit protobuf field".into());
            }
        }
        2 => {
            let _ = take_len_payload(buf, i)?;
        }
        5 => {
            *i = i.checked_add(4).ok_or("truncated 32-bit protobuf field")?;
            if *i > buf.len() {
                return Err("truncated 32-bit protobuf field".into());
            }
        }
        other => return Err(format!("unsupported protobuf wire type {other}")),
    }
    Ok(())
}

fn read_varint(buf: &[u8]) -> Result<(u64, usize), String> {
    let mut value = 0u64;
    for (n, byte) in buf.iter().copied().enumerate() {
        if n >= 10 {
            return Err("protobuf varint too long".into());
        }
        value |= u64::from(byte & 0x7f) << (7 * n);
        if byte & 0x80 == 0 {
            return Ok((value, n + 1));
        }
    }
    Err("truncated protobuf varint".into())
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[derive(Clone, PartialEq, Message)]
    struct NetCode {
        #[prost(int32, tag = "1")]
        value: i32,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Net {
        #[prost(message, optional, tag = "1")]
        code: Option<NetCode>,
        #[prost(string, tag = "2")]
        name: String,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Pad {
        #[prost(string, tag = "1")]
        id: String,
        #[prost(string, tag = "3")]
        number: String,
        #[prost(message, optional, tag = "4")]
        net: Option<Net>,
        #[prost(int32, tag = "5")]
        pad_type: i32,
    }

    #[test]
    fn splice_sets_net_without_dropping_other_fields() {
        let original = Pad {
            id: "abc".into(),
            number: "2".into(),
            net: None,
            pad_type: 2,
        };
        let spliced = set_len_field(&original.encode_to_vec(), 4, &encode_net_name("5V")).unwrap();
        let decoded = Pad::decode(spliced.as_slice()).unwrap();
        assert_eq!(decoded.id, "abc");
        assert_eq!(decoded.number, "2");
        assert_eq!(decoded.pad_type, 2);
        assert_eq!(decoded.net.as_ref().unwrap().name, "5V");
        assert_eq!(decoded.net.unwrap().code.unwrap().value, 0);
    }

    #[test]
    fn splice_sets_unconnected() {
        let original = Pad {
            id: "x".into(),
            number: "7".into(),
            net: Some(Net {
                code: Some(NetCode { value: 3 }),
                name: "GND".into(),
            }),
            pad_type: 1,
        };
        let spliced = set_len_field(&original.encode_to_vec(), 4, &encode_unconnected()).unwrap();
        let decoded = Pad::decode(spliced.as_slice()).unwrap();
        assert_eq!(decoded.net.as_ref().unwrap().name, "unconnected");
        assert_eq!(decoded.net.unwrap().code.unwrap().value, 0);
        assert_eq!(decoded.number, "7");
    }

    #[test]
    fn splice_replaces_existing_net() {
        let original = Pad {
            id: "x".into(),
            number: "1".into(),
            net: Some(Net {
                code: None,
                name: "old".into(),
            }),
            pad_type: 1,
        };
        let spliced = set_len_field(&original.encode_to_vec(), 4, &encode_net_name("GND")).unwrap();
        let decoded = Pad::decode(spliced.as_slice()).unwrap();
        assert_eq!(decoded.net.unwrap().name, "GND");
        assert_eq!(decoded.id, "x");
    }

    #[test]
    fn encode_net_includes_code() {
        let n = Net::decode(encode_net("5V", 7).as_slice()).unwrap();
        assert_eq!(n.name, "5V");
        assert_eq!(n.code.unwrap().value, 7);
    }

    #[derive(Clone, PartialEq, Message)]
    struct FootprintDef {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(bytes, repeated, tag = "11")]
        items: Vec<Vec<u8>>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct FootprintInst {
        #[prost(string, tag = "1")]
        id: String,
        #[prost(message, optional, tag = "6")]
        definition: Option<FootprintDef>,
    }

    #[test]
    fn map_len_fields_patches_nested_bytes() {
        let inst = FootprintInst {
            id: "fp1".into(),
            definition: Some(FootprintDef {
                name: "LED".into(),
                items: vec![b"pad-a".to_vec(), b"pad-b".to_vec()],
            }),
        };
        let patched = map_len_fields(&inst.encode_to_vec(), 6, |def| {
            map_len_fields(def, 11, |item| {
                if item == b"pad-a" {
                    Ok(b"pad-a-net".to_vec())
                } else {
                    Ok(item.to_vec())
                }
            })
        })
        .unwrap();
        let decoded = FootprintInst::decode(patched.as_slice()).unwrap();
        assert_eq!(decoded.id, "fp1");
        let def = decoded.definition.unwrap();
        assert_eq!(def.name, "LED");
        assert_eq!(def.items, vec![b"pad-a-net".to_vec(), b"pad-b".to_vec()]);
    }
}
