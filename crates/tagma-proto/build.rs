//! `tagma.v1`'s prost types, generated into `OUT_DIR` from
//! `proto/tagma/v1/tag.proto`. Generated code is never committed.

fn main() -> std::io::Result<()> {
    let proto = "../../proto/tagma/v1/tag.proto";
    println!("cargo:rerun-if-changed={proto}");
    println!("cargo:rerun-if-env-changed=PROTOC");
    prost_build::Config::new()
        // `prost::Name`, so a consumer that packs a Tag into an `Any` (or
        // routes by type URL) gets its full name.
        .enable_type_names()
        .compile_protos(&[proto], &["../../proto"])
}
