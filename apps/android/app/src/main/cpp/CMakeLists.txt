cmake_minimum_required(VERSION 3.22)
project(forwiroh_android C)
get_filename_component(ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../../../.." ABSOLUTE)
if(ANDROID_ABI STREQUAL "arm64-v8a")
    set(RUST_TARGET aarch64-linux-android)
elseif(ANDROID_ABI STREQUAL "x86_64")
    set(RUST_TARGET x86_64-linux-android)
else()
    message(FATAL_ERROR "Unsupported ABI: ${ANDROID_ABI}")
endif()
find_program(CARGO cargo REQUIRED)
get_filename_component(CLANG_DIR "${CMAKE_C_COMPILER}" DIRECTORY)
string(REPLACE "android-" "" ANDROID_API "${ANDROID_PLATFORM}")
set(RUST_LINKER "${CLANG_DIR}/${RUST_TARGET}${ANDROID_API}-clang")
set(RUST_DIR "${ROOT}/target/android/${ANDROID_ABI}")
set(RUST_LIB "${RUST_DIR}/${RUST_TARGET}/release/libforwiroh.a")
file(GLOB_RECURSE RUST_SOURCES CONFIGURE_DEPENDS "${ROOT}/crates/forwiroh/src/*.rs")
string(TOUPPER "${RUST_TARGET}" TARGET_ENV)
string(REPLACE "-" "_" TARGET_ENV "${TARGET_ENV}")
# Emit only staticlib so Rust completes LTO before the NDK's native JNI link.
add_custom_command(OUTPUT "${RUST_LIB}"
    COMMAND ${CMAKE_COMMAND} -E env
        "CARGO_TARGET_DIR=${RUST_DIR}"
        "CARGO_TARGET_${TARGET_ENV}_LINKER=${RUST_LINKER}"
        "CC_${RUST_TARGET}=${CMAKE_C_COMPILER}"
        "CFLAGS_${RUST_TARGET}=--target=${CMAKE_C_COMPILER_TARGET} --sysroot=${CMAKE_SYSROOT}"
        "AR_${RUST_TARGET}=${CMAKE_AR}"
        ${CARGO} rustc --manifest-path "${ROOT}/Cargo.toml" -p forwiroh --lib --crate-type staticlib
        --locked --release --target ${RUST_TARGET}
    DEPENDS ${RUST_SOURCES} "${ROOT}/Cargo.lock" "${ROOT}/Cargo.toml" "${ROOT}/crates/forwiroh/Cargo.toml"
    WORKING_DIRECTORY "${ROOT}" VERBATIM)
add_custom_target(rust_core DEPENDS "${RUST_LIB}")
add_library(forwiroh_jni SHARED bridge.c)
add_dependencies(forwiroh_jni rust_core)
target_include_directories(forwiroh_jni SYSTEM PRIVATE "${ROOT}/apps/ios/Shared")
target_compile_options(forwiroh_jni PRIVATE -Wall -Wextra -Werror)
target_link_options(forwiroh_jni PRIVATE "-Wl,-z,max-page-size=16384")
target_link_libraries(forwiroh_jni PRIVATE "${RUST_LIB}" android log dl m)
