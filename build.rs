fn main() {
    // sqlx::migrate! embeds the migrations at compile time, so a change must trigger a rebuild
    println!("cargo:rerun-if-changed=migrations");
}
