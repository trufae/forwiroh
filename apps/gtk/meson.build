project('forwiroh-gtk', ['vala', 'c'], version: '0.1.1', meson_version: '>=0.61')
gtk = dependency('gtk4', version: '>=4.8')
json = dependency('json-glib-1.0', version: '>=1.6')
threads = dependency('threads')
math = meson.get_compiler('c').find_library('m')
repo = meson.project_source_root() / '../..'
rust_dir = repo / 'target/release'
rust = meson.get_compiler('c').find_library('forwiroh', dirs: rust_dir)
headers = include_directories('../../crates/forwiroh/include')
core_sources = files('src/forwiroh.vapi', 'src/core.vala')
executable('forwiroh-gtk', core_sources + files('src/main.vala', 'src/tcp-client.vala'),
  dependencies: [gtk, json, threads, rust, math], include_directories: headers,
  build_rpath: rust_dir,
  install_rpath: get_option('prefix') / get_option('libdir'), install: true)
if host_machine.system() == 'linux'
  install_data(rust_dir / 'libforwiroh.so', install_dir: get_option('libdir'))
endif
install_data('org.radare.forwiroh.gtk.desktop', install_dir: get_option('datadir') / 'applications')
smoke = executable('core-smoke', core_sources + files('tests/core-smoke.vala'),
  dependencies: [json, threads, rust], include_directories: headers, build_rpath: rust_dir)
test('desktop-core', smoke, timeout: 60)
