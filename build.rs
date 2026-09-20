fn main() -> Result<(), embed_resource::CompilationResult> {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_required()
}
