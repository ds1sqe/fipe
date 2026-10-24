#include "fipe.h"
#include <string.h>
#include "interface.h"

/* The example serializes all provider calls, including its one unit test. */
static size_t calls;

static int named(FipeBytes name, const char *label) {
    size_t len = strlen(label);
    return name.len == len && name.data != NULL &&
           memcmp(name.data, label, len) == 0;
}

static FipeValue failure(const char *message) {
    FipeValue value = {0};
    value.tag = FIPE_TEXT;
    value.bytes.data = (const uint8_t *)message;
    value.bytes.len = strlen(message);
    return value;
}

static uint32_t call(FipeBytes name, const FipeValue *args, size_t len,
                     FipeEmit emit, void *context) {
    ++calls;
    FipeValue value = failure("invalid request");
    uint32_t status = 1;
    if (len == 1 && args != NULL) {
        if (named(name, "integer") && args[0].tag == FIPE_INTEGER) {
            value = args[0];
            status = 0;
        } else if (named(name, "text") && args[0].tag == FIPE_TEXT) {
            value = args[0];
            status = 0;
        } else if (named(name, "fail") && args[0].tag == FIPE_UNIT) {
            value = failure("provider refused");
        }
    }
    /* Arguments and static error text remain live until emit has copied them. */
    return emit(context, &value) == 0 ? status : 2;
}

FipeProvider linked_c_descriptor(void) {
    FipeProvider provider = {0};
    provider.version = FIPE_ABI_VERSION;
    provider.size = sizeof(FipeProvider);
    provider.module.data = (const uint8_t *)"proof.ops";
    provider.module.len = sizeof("proof.ops") - 1;
    provider.declarations.data = declarations;
    provider.declarations.len = sizeof(declarations);
    provider.call = call;
    return provider;
}

size_t linked_c_calls(void) { return calls; }
