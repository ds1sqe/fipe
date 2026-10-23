#include "fipe.h"
#include <string.h>
#include <stdalign.h>

static size_t calls;
static const char declarations[] =
    "pub unit: () -> ()\n"
    "pub integer: Integer -> Integer\n"
    "pub float: F64 -> F64\n"
    "pub text: String -> String\n"
    "pub choose: Integer -> String -> String\n"
    "pub fail: () -> Integer\n"
    "pub wrong: () -> Integer\n";

static int named(FipeBytes name, const char *expected) {
    size_t len = strlen(expected);
    return name.len == len && memcmp(name.data, expected, len) == 0;
}
static FipeValue text(const char *message) {
    FipeValue value = {0};
    value.tag = FIPE_TEXT;
    value.bytes.data = (const uint8_t *)message;
    value.bytes.len = strlen(message);
    return value;
}
static uint32_t call(FipeBytes name, const FipeValue *args, size_t len,
                     FipeEmit emit, void *context) {
    ++calls;
    FipeValue value = {0};
    uint32_t status = 0;
    uint8_t copied_text[256] = {0};
    size_t copied_len = 0;
    if (named(name, "unit") && len == 1 && args[0].tag == FIPE_UNIT) {
        value = args[0];
    } else if (named(name, "integer") && len == 1 && args[0].tag == FIPE_INTEGER) {
        value = args[0];
    } else if (named(name, "float") && len == 1 && args[0].tag == FIPE_FLOAT) {
        value = args[0];
    } else if (named(name, "text") && len == 1 && args[0].tag == FIPE_TEXT && args[0].bytes.len <= sizeof(copied_text)) {
        copied_len = args[0].bytes.len;
        if (copied_len != 0) memcpy(copied_text, args[0].bytes.data, copied_len);
        value = args[0];
        value.bytes.data = copied_text;
    } else if (named(name, "choose") && len == 2 && args[0].tag == FIPE_INTEGER && args[1].tag == FIPE_TEXT) {
        value = args[1];
    } else if (named(name, "wrong") && len == 1 && args[0].tag == FIPE_UNIT) {
        value = text("not an integer");
    } else {
        status = 1;
        value = text(named(name, "fail") && len == 1 && args[0].tag == FIPE_UNIT
                     ? "provider refused" : "unknown request");
    }
    (void)emit(context, &value);
    /* Emit must copy before this valid local storage is overwritten. */
    volatile uint8_t *erase = copied_text;
    for (size_t i = 0; i < copied_len; ++i) erase[i] = 0xff;
    return status;
}
FipeProvider proof_c_descriptor(void) {
    FipeProvider provider = {0};
    provider.version = FIPE_ABI_VERSION;
    provider.size = sizeof(FipeProvider);
    provider.module.data = (const uint8_t *)"proof.ops";
    provider.module.len = sizeof("proof.ops") - 1;
    provider.declarations.data = (const uint8_t *)declarations;
    provider.declarations.len = sizeof(declarations) - 1;
    provider.call = call;
    return provider;
}
FipeProvider proof_c_bad_version(void) {
    FipeProvider provider = proof_c_descriptor();
    provider.version = 1;
    return provider;
}
FipeProvider proof_c_bad_size(void) {
    FipeProvider provider = proof_c_descriptor();
    --provider.size;
    return provider;
}
size_t proof_c_calls(void) { return calls; }
size_t proof_c_layout(size_t index) {
    const size_t fields[] = {
        sizeof(FipeBytes), alignof(FipeBytes), offsetof(FipeBytes, data), offsetof(FipeBytes, len),
        sizeof(FipeValue), alignof(FipeValue), offsetof(FipeValue, tag), offsetof(FipeValue, integer),
        offsetof(FipeValue, floating), offsetof(FipeValue, bytes),
        sizeof(FipeProvider), alignof(FipeProvider), offsetof(FipeProvider, version),
        offsetof(FipeProvider, size), offsetof(FipeProvider, module), offsetof(FipeProvider, declarations),
        offsetof(FipeProvider, call), FIPE_ABI_VERSION, FIPE_UNIT, FIPE_INTEGER, FIPE_FLOAT, FIPE_TEXT
    };
    return index < sizeof(fields) / sizeof(fields[0]) ? fields[index] : SIZE_MAX;
}
