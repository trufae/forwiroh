#ifndef FORWIROH_H
#define FORWIROH_H
#include <stdint.h>
#include <stddef.h>
typedef struct Handle ForwirohHandle;
/* Version is available without opening the core. Static string; do not free. */
const char * _Nonnull forwiroh_version(void);
/* Public crypto capabilities as owned JSON; release with string_free. */
char * _Nonnull forwiroh_crypto_info(void);
/* All functions block: call off the UI thread. Strings returned are owned. */
ForwirohHandle *forwiroh_open(const char *path, const char *name, const uint8_t key[32], char **error);
char *forwiroh_command(ForwirohHandle *, const char *json);
char *forwiroh_next_event(ForwirohHandle *);
int32_t forwiroh_send_packet(ForwirohHandle *, const char *session, const uint8_t *, size_t);
int32_t forwiroh_receive_packet(ForwirohHandle *, const char *session, uint8_t *, size_t);
void forwiroh_string_free(char *);
/* shutdown wakes blocked calls. Join ALL callers before free. */
void forwiroh_shutdown(ForwirohHandle *);
void forwiroh_free(ForwirohHandle *);
#endif
