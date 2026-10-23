//! Borrow scalar arguments and copy scalar replies; no resource/tree conversion.
use crate::{abi, Error};
use fipe::{
    ir::Type,
    value::{Prelude, Value},
};
use std::mem::{align_of, size_of};

/// Pointer provenance and allocation validity remain the provider's obligation.
pub unsafe fn slice<'a, T>(
    data: *const T,
    len: usize,
) -> Result<&'a [T], Error> {
    if len == 0 {
        return Ok(&[]);
    }
    if data.is_null()
        || !(data as usize).is_multiple_of(align_of::<T>())
        || len > (isize::MAX as usize) / size_of::<T>()
    {
        return Err(Error::Contract("invalid native slice"));
    }
    Ok(unsafe { std::slice::from_raw_parts(data, len) })
}
pub unsafe fn text<'a>(bytes: abi::Bytes) -> Result<&'a str, Error> {
    std::str::from_utf8(unsafe { slice(bytes.data, bytes.len)? })
        .map_err(|_| Error::Contract("native text is not UTF-8"))
}

pub fn tag(ty: &Type, prelude: &Prelude) -> Result<u32, Error> {
    match ty {
        Type::Unit => Ok(abi::UNIT),
        Type::Meaning { index, arguments } if arguments.is_empty() => {
            if Some(*index) == prelude.integer {
                Ok(abi::INTEGER)
            } else if Some(*index) == prelude.f64 {
                Ok(abi::FLOAT)
            } else if Some(*index) == prelude.string {
                Ok(abi::TEXT)
            } else {
                Err(Error::Contract("unsupported native declaration type"))
            }
        }
        _ => Err(Error::Contract("unsupported native declaration type")),
    }
}

pub fn encode(value: &Value, tag: u32) -> Result<abi::Value, Error> {
    match (value, tag) {
        (Value::Unit, abi::UNIT) => Ok(abi::Value::default()),
        (Value::Int(n), abi::INTEGER) => Ok(abi::Value::integer(*n)),
        (Value::Big(_), abi::INTEGER) => {
            Err(Error::Contract("native Integer argument exceeds i64 range"))
        }
        (Value::F64(n), abi::FLOAT) if n.is_finite() => {
            Ok(abi::Value::float(*n))
        }
        (Value::Text(text), abi::TEXT) => Ok(abi::Value::text(text)),
        _ => Err(Error::Contract("invalid native scalar argument")),
    }
}

pub unsafe fn decode(value: &abi::Value) -> Result<Value, Error> {
    match value.tag {
        abi::UNIT => Ok(Value::Unit),
        abi::INTEGER => Ok(Value::Int(value.integer)),
        abi::FLOAT if value.float.is_finite() => Ok(Value::F64(value.float)),
        abi::TEXT => Ok(Value::text(unsafe { text(value.bytes)? })),
        _ => Err(Error::Contract("unknown native tag or nonfinite float")),
    }
}
