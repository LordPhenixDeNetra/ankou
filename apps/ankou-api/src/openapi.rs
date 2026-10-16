use crate::routes::{auth, devices, servers};
use utoipa::OpenApi;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

#[derive(OpenApi)]
#[openapi(
    info(title = "aNkou API", version = "1"),
    paths(auth::register, auth::login, auth::refresh, auth::logout, auth::me,
          servers::list, devices::enroll, devices::list, devices::revoke),
    modifiers(&Bearer)
)]
pub struct ApiDoc;

struct Bearer;

impl utoipa::Modify for Bearer {
    fn modify(&self, doc: &mut utoipa::openapi::OpenApi) {
        if let Some(c) = doc.components.as_mut() {
            c.add_security_scheme(
                "bearer",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
        }
    }
}
