extern crate pkg_config;

fn main() {
    pkg_config::probe_library("sdl2").unwrap();
    pkg_config::probe_library("gstreamer-1.0").unwrap();
}