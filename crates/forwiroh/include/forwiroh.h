#ifndef FORWIROH_H
#define FORWIROH_H
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct Handle ForwirohHandle;
/* Static version string. Do not free it. */
const char *forwiroh_version(void);
/* Owned JSON strings: release with forwiroh_string_free. */
char *forwiroh_crypto_info(void);
char *forwiroh_qr_code(const char *text);
/* All calls block. Use worker threads, including opening and shutdown.
 * Desktop open persists an owner-only identity in path; only one process can
 * own that state directory. Mobile open uses an external 32-byte Keychain key. */
ForwirohHandle *forwiroh_open_desktop(const char *path, const char *name, char **error);
ForwirohHandle *forwiroh_open(const char *path, const char *name, const uint8_t key[32], char **error);
char *forwiroh_command(ForwirohHandle *, const char *json);
char *forwiroh_next_event(ForwirohHandle *);
int32_t forwiroh_send_packet(ForwirohHandle *, const char *session, const uint8_t *, size_t);
int32_t forwiroh_receive_packet(ForwirohHandle *, const char *session, uint8_t *, size_t);
void forwiroh_string_free(char *);
/* Shutdown wakes blocked calls. Join ALL callers before free. */
void forwiroh_shutdown(ForwirohHandle *);
void forwiroh_free(ForwirohHandle *);
#ifdef __cplusplus
}
#endif
#endif
