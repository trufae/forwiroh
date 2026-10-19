#include <jni.h>
#include <poll.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include "forwiroh.h"

#define JNI(name) Java_org_forwiroh_RustCore_##name
#define HANDLE(value) ((ForwirohHandle *)(intptr_t)(value))
#define UNUSED() (void)klass

static void fail(JNIEnv *env, const char *message) {
    jclass type = (*env)->FindClass(env, "java/lang/IllegalStateException");
    if (type) (*env)->ThrowNew(env, type, message);
}
/* Use standard UTF-8 byte arrays, not JNI's Modified UTF-8 (emoji differ). */
static char *text(JNIEnv *env, jbyteArray value) {
    if (!value) { fail(env, "Missing input"); return NULL; }
    jsize n = (*env)->GetArrayLength(env, value);
    if (n > 8192) { fail(env, "Input too large"); return NULL; }
    char *result = calloc((size_t)n + 1, 1);
    if (!result) { fail(env, "Out of memory"); return NULL; }
    (*env)->GetByteArrayRegion(env, value, 0, n, (jbyte *)result);
    if ((*env)->ExceptionCheck(env)) { free(result); return NULL; }
    if (memchr(result, 0, (size_t)n)) { free(result); fail(env, "NUL in input"); return NULL; }
    return result;
}
static jbyteArray bytes(JNIEnv *env, const char *value) {
    if (!value) return NULL;
    jsize n = (jsize)strlen(value);
    jbyteArray result = (*env)->NewByteArray(env, n);
    if (result) (*env)->SetByteArrayRegion(env, result, 0, n, (const jbyte *)value);
    return result;
}
static jbyteArray owned(JNIEnv *env, char *value) {
    jbyteArray result = bytes(env, value);
    forwiroh_string_free(value);
    return result;
}
extern void forwiroh_android_init(JNIEnv *env, jobject context);
JNIEXPORT void JNICALL JNI(initializeNative)(JNIEnv *env, jclass klass, jobject context) {
    UNUSED(); forwiroh_android_init(env, context);
}
JNIEXPORT jlong JNICALL JNI(openNative)(JNIEnv *env, jclass klass, jbyteArray path, jbyteArray key) {
    UNUSED();
    if (!key || (*env)->GetArrayLength(env, key) != 32) { fail(env, "Invalid identity"); return 0; }
    char *directory = text(env, path);
    if (!directory) return 0;
    uint8_t secret[32];
    (*env)->GetByteArrayRegion(env, key, 0, 32, (jbyte *)secret);
    char *error = NULL;
    ForwirohHandle *handle = forwiroh_open(directory, "Android", secret, &error);
    volatile uint8_t *wipe = secret;
    for (size_t i = 0; i < sizeof(secret); i++) wipe[i] = 0;
    free(directory);
    if (!handle) fail(env, error ? error : "Cannot open core");
    forwiroh_string_free(error);
    return (jlong)(intptr_t)handle;
}
JNIEXPORT jbyteArray JNICALL JNI(commandNative)(JNIEnv *env, jclass klass, jlong handle, jbyteArray json) {
    UNUSED();
    char *input = text(env, json);
    if (!input) return NULL;
    char *output = forwiroh_command(HANDLE(handle), input);
    free(input);
    return owned(env, output);
}
JNIEXPORT jbyteArray JNICALL JNI(eventNative)(JNIEnv *env, jclass klass, jlong handle) {
    UNUSED(); return owned(env, forwiroh_next_event(HANDLE(handle)));
}
JNIEXPORT jbyteArray JNICALL JNI(versionNative)(JNIEnv *env, jclass klass) {
    UNUSED(); return bytes(env, forwiroh_version());
}
JNIEXPORT void JNICALL JNI(shutdownNative)(JNIEnv *env, jclass klass, jlong handle) {
    (void)env; UNUSED(); forwiroh_shutdown(HANDLE(handle));
}
JNIEXPORT void JNICALL JNI(freeNative)(JNIEnv *env, jclass klass, jlong handle) {
    (void)env; UNUSED(); forwiroh_free(HANDLE(handle));
}
JNIEXPORT jint JNICALL JNI(sendNative)(JNIEnv *env, jclass klass, jlong handle, jbyteArray session, jbyteArray packet, jint length) {
    UNUSED();
    if (!packet || length < 0 || length > 1100 || length > (*env)->GetArrayLength(env, packet)) return -1;
    char *id = text(env, session);
    if (!id) return -1;
    uint8_t data[1100];
    (*env)->GetByteArrayRegion(env, packet, 0, length, (jbyte *)data);
    int result = forwiroh_send_packet(HANDLE(handle), id, data, (size_t)length);
    free(id);
    return result;
}
JNIEXPORT jbyteArray JNICALL JNI(receiveNative)(JNIEnv *env, jclass klass, jlong handle, jbyteArray session) {
    UNUSED();
    char *id = text(env, session);
    if (!id) return NULL;
    uint8_t data[1100];
    int n = forwiroh_receive_packet(HANDLE(handle), id, data, sizeof(data));
    free(id);
    if (n <= 0) return NULL;
    jbyteArray result = (*env)->NewByteArray(env, n);
    if (result) (*env)->SetByteArrayRegion(env, result, 0, n, (const jbyte *)data);
    return result;
}
/* A bounded poll lets the VPN owner join the reader before closing/reusing fd. */
JNIEXPORT jint JNICALL JNI(readTun)(JNIEnv *env, jclass klass, jint fd, jbyteArray packet) {
    UNUSED();
    if (!packet || (*env)->GetArrayLength(env, packet) < 65535) return -1;
    struct pollfd p = { .fd = fd, .events = POLLIN };
    int ready = poll(&p, 1, 250);
    if (ready == 0 || (ready < 0 && errno == EINTR)) return 0;
    if (ready < 0 || (p.revents & (POLLERR | POLLHUP | POLLNVAL))) return -1;
    uint8_t data[65535];
    ssize_t n = read(fd, data, sizeof(data));
    if (n < 0 && (errno == EINTR || errno == EAGAIN)) return 0;
    if (n <= 0) return -1;
    (*env)->SetByteArrayRegion(env, packet, 0, (jsize)n, (const jbyte *)data);
    return (jint)n;
}
