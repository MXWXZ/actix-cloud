use actix_cloud::response::generate_response;

fn main() {
    generate_response("response", "response.rs").unwrap();
}
