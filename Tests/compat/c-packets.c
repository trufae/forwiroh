#include "forwiroh.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    assert(argc == 2);
    assert(FORWIROH_C_ABI_VERSION == 1 && FORWIROH_CONTROL_API_VERSION == 1);
    assert(forwiroh_max_packet_size() == FORWIROH_MAX_PACKET_SIZE);
    char *error = NULL;
    ForwirohHandle *core = forwiroh_open_desktop(argv[1], "C consumer", &error);
    assert(core && !error);
    char *info = forwiroh_command(core, "{\"op\":\"api_info\"}");
    assert(info && strstr(info, "\"control_api_version\":1") && strstr(info, "\"c_abi_version\":1"));
    assert(strstr(info, "vpn_packet"));
    forwiroh_string_free(info);
    uint8_t *packet = (uint8_t *)malloc(FORWIROH_MAX_PACKET_SIZE);
    assert(packet);
    assert(forwiroh_receive_packet_ex(core, "missing", packet, 1280, &error) == -1);
    assert(error && strstr(error, "buffer_too_small"));
    forwiroh_string_free(error);
    error = NULL;
    assert(forwiroh_receive_packet_ex(core, "missing", packet, FORWIROH_MAX_PACKET_SIZE, &error) == -1);
    assert(error && strstr(error, "session_not_found"));
    forwiroh_string_free(error);
    error = NULL;
    assert(forwiroh_send_packet_ex(core, "missing", packet, 0, &error) == -1);
    assert(error && strstr(error, "invalid_packet"));
    forwiroh_string_free(error);
    assert(forwiroh_receive_packet(core, "missing", packet, 1280) == -1);
    assert(forwiroh_send_packet(core, "missing", packet, 0) == -1);
    free(packet);
    forwiroh_shutdown(core);
    forwiroh_free(core);
    return 0;
}
