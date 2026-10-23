//! Optional native providers, independent of Standard and the OS.
//!
//! The embedding driver retains permission decisions and delivers the result
//! through `Machine::reply`, which validates it against the Fipe declaration.
//! Only trusted native code may implement the unsafe ABI contract.
pub mod abi;
mod values;

use fipe::{
    decl::{self, Declarations},
    embed::Request,
    ir::{Tables, Type},
    value::{Prelude, Value},
};
use std::{ffi::c_void, fmt, marker::PhantomData, rc::Rc};

/// A provider registration, conversion, or invocation failure.
#[derive(Debug)]
pub enum Error {
    Contract(&'static str),
    Declarations(Vec<decl::Error>),
    Host(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(message) => f.write_str(message),
            Self::Declarations(errors) => {
                write!(f, "invalid provider declarations: {errors:?}")
            }
            Self::Host(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}

/// A borrowed, synchronous provider. It is intentionally neither Send nor Sync.
/// Resource ownership and concurrent execution await their upstream contracts.
pub struct Provider<'a> {
    descriptor: &'a abi::Provider,
    module: String,
    source: String,
    _local: PhantomData<Rc<()>>,
}
impl<'a> Provider<'a> {
    /// Validate a descriptor without invoking provider code.
    ///
    /// # Safety
    /// The descriptor and its buffers must be immutable and valid for `'a`.
    /// Nonempty nonnull buffers admitted by metadata checks must designate
    /// initialized storage in one live allocation. `call` must validate function
    /// labels, argument counts and tags before using inputs. It emits once,
    /// synchronously on the invoking thread, without overlapping/reentrant emit,
    /// retaining inputs/context/emitter, or unwinding across either callback.
    /// Every emitted Value field, including inactive fields, is initialized;
    /// padding need not be. Only selected text needs live backing storage.
    /// Emitted storage stays valid/aligned until emit returns. Shared mutable
    /// state must be safely reentrant or externally serialized across ALL
    /// wrappers; this wrapper's !Send/!Sync marker is insufficient for that.
    /// Code remains loaded for `'a`. Foreign code is trusted; metadata checks
    /// cannot establish pointer provenance or allocation liveness.
    ///
    /// # Errors
    /// Unsupported version/size, missing callback, invalid UTF-8/declarations.
    pub unsafe fn new(descriptor: &'a abi::Provider) -> Result<Self, Error> {
        if descriptor.version != abi::VERSION
            || descriptor.size != size_of::<abi::Provider>()
        {
            return Err(Error::Contract(
                "unsupported provider ABI version or size",
            ));
        }
        if descriptor.call.is_none() {
            return Err(Error::Contract("provider has no call function"));
        }
        let module = unsafe { values::text(descriptor.module)? }.to_owned();
        let source =
            unsafe { values::text(descriptor.declarations)? }.to_owned();
        Declarations::new()
            .module(&module, &source)
            .map_err(Error::Declarations)?;
        Ok(Self {
            descriptor,
            module,
            source,
            _local: PhantomData,
        })
    }

    /// Register the checked API without replacing another host's module.
    ///
    /// # Errors
    /// An existing module or declarations incompatible with this host.
    pub fn register(
        &self,
        declarations: &mut Declarations,
    ) -> Result<(), Error> {
        if declarations.get_module(&self.module).is_some() {
            return Err(Error::Contract(
                "provider module is already registered",
            ));
        }
        declarations
            .module(&self.module, &self.source)
            .map_err(Error::Declarations)
    }

    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }
    #[must_use]
    pub fn declarations(&self) -> &str {
        &self.source
    }

    /// Invoke after the driver has authorized the call and its environment.
    /// A returned value must still pass through the embedding reply validator.
    ///
    /// # Errors
    /// A different module, unsupported values, malformed reply, or host Fault.
    pub fn call(
        &mut self,
        request: &Request,
        tables: &Tables,
    ) -> Result<Value, Error> {
        let Request::Call {
            declaration,
            arguments,
            ..
        } = request
        else {
            return Err(Error::Contract(
                "a provider answers calls, not file requests",
            ));
        };
        let declared = tables
            .declarations
            .get(*declaration)
            .ok_or(Error::Contract("unknown host declaration"))?;
        if tables.namespaces.get(declared.namespace) != Some(&self.module) {
            return Err(Error::Contract(
                "call belongs to a different provider",
            ));
        }
        let prelude = Prelude::new(tables);
        let mut ty = &declared.ty;
        let mut tags = Vec::new();
        while let Type::Function { parameter, result } = ty {
            tags.push(values::tag(parameter, &prelude)?);
            ty = result;
        }
        // Opaque results must be refused before even a scalar-only callback runs.
        values::tag(ty, &prelude)?;
        if tags.len() != arguments.len() {
            return Err(Error::Contract("wrong native argument count"));
        }
        let arguments = arguments
            .iter()
            .zip(tags)
            .map(|(value, tag)| values::encode(value, tag))
            .collect::<Result<Vec<_>, _>>()?;
        let mut reply = Reply {
            value: None,
            duplicate: false,
        };
        // Registration establishes callback validity; all borrowed storage lives through emit.
        let status = unsafe {
            self.descriptor
                .call
                .ok_or(Error::Contract("provider has no call function"))?(
                abi::Bytes::new(declared.label.as_bytes()),
                arguments.as_ptr(),
                arguments.len(),
                emit,
                (&raw mut reply).cast(),
            )
        };
        if reply.duplicate {
            return Err(Error::Contract(
                "provider emitted more than one reply",
            ));
        }
        let value = reply
            .value
            .ok_or(Error::Contract("provider emitted no reply"))??;
        match (status, value) {
            (0, value) => Ok(value),
            (1, Value::Text(message)) => Err(Error::Host((*message).clone())),
            (1, _) => Err(Error::Contract("a host Fault must be UTF-8 text")),
            _ => Err(Error::Contract("unknown provider call status")),
        }
    }
}

struct Reply {
    value: Option<Result<Value, Error>>,
    duplicate: bool,
}

unsafe extern "C" fn emit(
    context: *mut c_void,
    value: *const abi::Value,
) -> u32 {
    // The context is created by call and is never exposed beyond its duration.
    let reply = unsafe { &mut *context.cast::<Reply>() };
    if reply.value.is_some() {
        reply.duplicate = true;
        return 1;
    }
    let result = unsafe { values::slice(value, 1) }
        .and_then(|values| unsafe { values::decode(&values[0]) });
    let status = u32::from(result.is_err());
    reply.value = Some(result);
    status
}
