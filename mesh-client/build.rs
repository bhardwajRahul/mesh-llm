use std::fs;
use std::path::Path;

// The canonical schema lives in the mesh-llm crate so the workspace has a
// single source of truth for the wire protocol. mesh-client compiles it
// from there rather than carrying its own copy.
const PROTO_REL_PATH: &str = "../mesh-llm/proto/node.proto";
const PROTO_INCLUDE_DIR: &str = "../mesh-llm/proto";

fn main() {
    watch_path(Path::new(PROTO_INCLUDE_DIR));
    compile_node_proto();
}

fn watch_path(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            watch_path(&entry.path());
        }
    }
}

fn compile_node_proto() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    std::env::set_var("PROTOC", protoc);

    prost_build::Config::new()
        .compile_protos(&[PROTO_REL_PATH], &[PROTO_INCLUDE_DIR])
        .expect("compile node proto");
}
