#ifndef FIPE_PROVIDER_H
#define FIPE_PROVIDER_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Experimental scalar layout. Version 1's tree layout is unsupported. */
#define FIPE_ABI_VERSION 2
#define FIPE_UNIT 0
#define FIPE_INTEGER 1
#define FIPE_FLOAT 2
#define FIPE_TEXT 3
/* UTF-8 bytes, not NUL-terminated; zero length permits NULL. */
typedef struct { const uint8_t *data; size_t len; } FipeBytes;
/* Initialize every field (for example FipeValue reply = {0};), then set tag
   and its payload. Padding need not be initialized. Pointer backing storage
   is required only for selected TEXT; inactive pointer fields may be NULL. */
typedef struct FipeValue {
    uint32_t tag;
    int64_t integer;
    double floating; /* finite F64 only */
    FipeBytes bytes;
} FipeValue;
/* emit copies one scalar synchronously on the invoking thread; no overlapping
   or reentrant emit. The value/text storage must remain valid and aligned until
   emit returns. Retain no inputs, context or emitter. 0=accepted, other=invalid. */
typedef uint32_t (*FipeEmit)(void *, const FipeValue *);
/* Validate function label, argument count and tags before accessing inputs.
   Emit exactly once. Return 0=scalar, 1=host Fault (emit TEXT); other statuses
   fail. No Rust unwinding or C++ exceptions may cross either callback boundary.
   Text lengths must fit PTRDIFF_MAX, with nonempty buffers in live allocations.
   INTEGER transports only Fipe Integer values in the signed int64_t range. */
typedef uint32_t (*FipeCall)(FipeBytes, const FipeValue *, size_t, FipeEmit, void *);
/* Descriptor/code/buffers remain immutable and valid for registration lifetime.
   Shared mutable provider state must be safely reentrant or externally serialized
   across ALL registrations; one thread-local wrapper does not guarantee this.
   Only trusted native code: pointer checks cannot prove allocation validity. */
typedef struct {
    uint32_t version;
    size_t size;
    FipeBytes module, declarations;
    FipeCall call;
} FipeProvider;
#ifdef __cplusplus
}
#endif
#endif
