fn main() {
    println!("cargo:rerun-if-changed=resources/resource.rc");
    println!("cargo:rerun-if-changed=resources/icon.ico");
    embed_resource::compile("resources/resource.rc", &[] as &[&str])
        .manifest_optional()
        .unwrap();
}
