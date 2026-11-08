#[path = "../../licenses/generate.rs"]
mod licenses;

fn main() {
    licenses::write_licenses(None);
}
