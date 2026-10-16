use ankou_server_domain::DomainError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("erreur PostgreSQL : {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration impossible : {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("adresse e-mail déjà utilisée")]
    EmailTaken,
    #[error("clé publique déjà enregistrée")]
    KeyTaken,
    #[error("introuvable")]
    NotFound,
}
