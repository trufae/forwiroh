/* Consumer of the frozen header from commit 93883c1. */
#include "forwiroh.h"
#include <assert.h>
#include <stdint.h>
#include <string.h>

int main(int argc, char **argv) {
    assert(argc == 2);
    assert(forwiroh_version() && *forwiroh_version());
    char *value = forwiroh_crypto_info();
    assert(value && strstr(value, "Ed25519"));
    forwiroh_string_free(value);
    value = forwiroh_qr_code("legacy consumer");
    assert(value && strstr(value, "width"));
    forwiroh_string_free(value);
    char *error = NULL;
    ForwirohHandle *core = forwiroh_open_desktop(argv[1], "legacy C", &error);
    assert(core && !error);
    value = forwiroh_command(core, "{\"op\":\"status\"}");
    assert(value && strstr(value, "legacy C") && strstr(value, "\"data\""));
    forwiroh_string_free(value);
    value = forwiroh_command(core, "{\"op\":\"pending\"}");
    assert(value && strstr(value, "\"data\":[]"));
    forwiroh_string_free(value);
    uint8_t packet[65535] = {0};
    assert(forwiroh_receive_packet(core, "missing", packet, sizeof(packet)) == -1);
    assert(forwiroh_send_packet(core, "missing", packet, 0) == -1);
    forwiroh_shutdown(core);
    /* Shutdown drains existing events before reporting end-of-stream. */
    while ((value = forwiroh_next_event(core)) != NULL) { forwiroh_string_free(value); }
    forwiroh_free(core);
    uint8_t key[32] = {17};
    core = forwiroh_open(argv[1], "legacy mobile", key, &error);
    assert(core && !error);
    forwiroh_shutdown(core);
    forwiroh_free(core);
    return 0;
}
