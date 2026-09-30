//! Small, bounded canonical CBOR subset. Definite arrays, unsigned ints and byte
//! strings only. Records have an exact arity and type tag: no ignored fields.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Uint(u64),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
}
fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=255 => out.extend([m | 24, n as u8]),
        256..=65535 => {
            out.push(m | 25);
            out.extend((n as u16).to_be_bytes());
        }
        65536..=4294967295 => {
            out.push(m | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend(n.to_be_bytes());
        }
    }
}
fn put(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Uint(n) => head(out, 0, *n),
        Value::Bytes(b) => {
            head(out, 2, b.len() as u64);
            out.extend(b);
        }
        Value::Array(a) => {
            head(out, 4, a.len() as u64);
            for v in a {
                put(v, out);
            }
        }
    }
}
pub(super) fn encode(v: &Value) -> Result<Vec<u8>> {
    fn bound(v: &Value, depth: usize) -> Result<usize> {
        ensure!(depth < 8, "CBOR nesting");
        match v {
            Value::Uint(_) => Ok(9),
            Value::Bytes(b) => {
                ensure!(b.len() <= 65527, "body too large");
                Ok(b.len() + 9)
            }
            Value::Array(a) => {
                ensure!(a.len() <= 256, "CBOR list too large");
                let mut n = 9;
                for v in a {
                    n += bound(v, depth + 1)?;
                    ensure!(n <= 65536, "body too large");
                }
                Ok(n)
            }
        }
    }
    let size = bound(v, 0)?;
    let mut out = Vec::with_capacity(size);
    put(v, &mut out);
    ensure!(out.len() <= 65536, "body too large");
    Ok(out)
}
fn take<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    ensure!(n <= input.len(), "truncated CBOR");
    let (a, b) = input.split_at(n);
    *input = b;
    Ok(a)
}
fn get(input: &mut &[u8], depth: usize) -> Result<Value> {
    ensure!(depth < 8, "CBOR nesting");
    let b = take(input, 1)?[0];
    let info = b & 31;
    let n = match info {
        0..=23 => info as u64,
        24 => take(input, 1)?[0] as u64,
        25 => u16::from_be_bytes(take(input, 2)?.try_into()?) as u64,
        26 => u32::from_be_bytes(take(input, 4)?.try_into()?) as u64,
        27 => u64::from_be_bytes(take(input, 8)?.try_into()?),
        _ => bail!("indefinite CBOR"),
    };
    match b >> 5 {
        0 => Ok(Value::Uint(n)),
        2 => Ok(Value::Bytes(take(input, usize::try_from(n)?)?.to_vec())),
        4 => {
            ensure!(n <= 256, "CBOR list too large");
            let mut a = Vec::new();
            for _ in 0..n {
                a.push(get(input, depth + 1)?);
            }
            Ok(Value::Array(a))
        }
        _ => bail!("unsupported CBOR type"),
    }
}
pub(super) fn decode(bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= 65536, "body too large");
    let mut input = bytes;
    let v = get(&mut input, 0)?;
    ensure!(
        input.is_empty() && encode(&v)? == bytes,
        "noncanonical/unknown trailing CBOR"
    );
    Ok(v)
}
pub(super) fn record(tag: &str, fields: &[&[u8]]) -> Result<Vec<u8>> {
    encode(&Value::Array(vec![
        Value::Uint(1),
        Value::Bytes(tag.as_bytes().to_vec()),
        Value::Array(fields.iter().map(|v| Value::Bytes(v.to_vec())).collect()),
    ]))
}
pub(super) fn fields(bytes: &[u8], tag: &str, arity: usize) -> Result<Vec<Vec<u8>>> {
    let Value::Array(v) = decode(bytes)? else {
        bail!("record array")
    };
    ensure!(
        v.len() == 3 && v[0] == Value::Uint(1) && v[1] == Value::Bytes(tag.as_bytes().to_vec()),
        "record version/purpose"
    );
    let Value::Array(a) = &v[2] else { bail!("field array") };
    ensure!(a.len() == arity, "unknown/missing field");
    a.iter()
        .map(|v| match v {
            Value::Bytes(b) => Ok(b.clone()),
            _ => bail!("field type"),
        })
        .collect()
}
