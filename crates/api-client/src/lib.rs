//! Client de l'API `/v1` d'aNkou : compte, session (jeton de rafraîchissement), serveurs, appareils.

mod device;
mod session;

pub use device::{load_servers, save_servers, servers_cache_path, to_entries};
pub use session::{Session, session_path};

use ankou_proto::PublicKey;
use ankou_proto::api::{
    ApiError, ApiErrorCode, EnrollDeviceRequest, EnrollDeviceResponse, LoginRequest,
    RefreshRequest, RegisterRequest, ServerInfo, TokenPair,
};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{code}: {1}", code = .0.as_str())]
    Api(ApiErrorCode, String),
    #[error("réseau : {0}")]
    Network(String),
    #[error("session expirée")]
    SessionExpired,
    #[error("fichier : {0}")]
    Io(String),
    #[error("aucun serveur disponible")]
    NoServer,
    #[error("serveur inconnu : {0}")]
    UnknownServer(String),
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network(e.to_string())
    }
}

const TIMEOUT: Duration = Duration::from_secs(15);

pub struct ApiClient {
    base: Url,
    http: reqwest::Client,
    session: Option<Session>,
    access_token: Option<String>,
}

/// `https` exigé ; `http` admis seulement vers la machine locale (tests, tunnel SSH).
fn parse_base(url: &str) -> Result<Url, ClientError> {
    let parsed = Url::parse(url).map_err(|e| ClientError::Network(format!("{url} : {e}")))?;
    let local = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    match parsed.scheme() {
        "https" => Ok(parsed),
        "http" if local => Ok(parsed),
        _ => Err(ClientError::Network(format!("{url} : https requis"))),
    }
}

impl ApiClient {
    pub fn new(url: &str) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder().timeout(TIMEOUT).build()?;
        Ok(Self {
            base: parse_base(url)?,
            http,
            session: None,
            access_token: None,
        })
    }

    pub fn with_session(session: Session) -> Result<Self, ClientError> {
        let mut client = Self::new(&session.api_url)?;
        client.session = Some(session);
        Ok(client)
    }

    /// Session courante, à sauvegarder après chaque appel (le jeton tourne).
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn set_access_token(&mut self, token: String) {
        self.access_token = Some(token);
    }

    pub async fn register(
        &mut self,
        invitation: &str,
        email: &str,
        password: &str,
    ) -> Result<&Session, ClientError> {
        let req = RegisterRequest {
            invitation_code: Some(invitation.to_owned()),
            email: email.to_owned(),
            password: password.to_owned(),
        };
        let pair = self
            .send(Method::POST, "v1/auth/register", Some(&req), None)
            .await?;
        Ok(self.store_tokens(pair))
    }

    pub async fn login(&mut self, email: &str, password: &str) -> Result<&Session, ClientError> {
        let req = LoginRequest {
            email: email.to_owned(),
            password: password.to_owned(),
        };
        let pair = self
            .send(Method::POST, "v1/auth/login", Some(&req), None)
            .await?;
        Ok(self.store_tokens(pair))
    }

    pub async fn servers(&mut self) -> Result<Vec<ServerInfo>, ClientError> {
        self.authed::<(), _>(Method::GET, "v1/servers", None).await
    }

    pub async fn enroll(
        &mut self,
        name: &str,
        public_key: &PublicKey,
    ) -> Result<EnrollDeviceResponse, ClientError> {
        let req = EnrollDeviceRequest {
            name: name.to_owned(),
            public_key: public_key.clone(),
        };
        self.authed(Method::POST, "v1/devices", Some(&req)).await
    }

    fn store_tokens(&mut self, pair: TokenPair) -> &Session {
        self.access_token = Some(pair.access_token);
        self.session.insert(Session {
            api_url: self.base.as_str().trim_end_matches('/').to_owned(),
            refresh_token: pair.refresh_token,
        })
    }

    async fn refresh(&mut self) -> Result<(), ClientError> {
        let Some(session) = &self.session else {
            return Err(ClientError::SessionExpired);
        };
        let req = RefreshRequest {
            refresh_token: session.refresh_token.clone(),
        };
        match self
            .send(Method::POST, "v1/auth/refresh", Some(&req), None)
            .await
        {
            Ok(pair) => {
                self.store_tokens(pair);
                Ok(())
            }
            Err(ClientError::Api(ApiErrorCode::AuthInvalid | ApiErrorCode::AuthExpired, _)) => {
                Err(ClientError::SessionExpired)
            }
            Err(e) => Err(e),
        }
    }

    /// Requête authentifiée : rafraîchit le jeton s'il manque, et une fois sur un 401.
    async fn authed<B: Serialize, T: DeserializeOwned>(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ClientError> {
        if self.access_token.is_none() {
            self.refresh().await?;
        }
        let first = self
            .send(method.clone(), path, body, self.access_token.as_deref())
            .await;
        match first {
            Err(ClientError::Api(ApiErrorCode::AuthInvalid | ApiErrorCode::AuthExpired, _)) => {
                self.refresh().await?;
                self.send(method, path, body, self.access_token.as_deref())
                    .await
            }
            other => other,
        }
    }

    async fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        token: Option<&str>,
    ) -> Result<T, ClientError> {
        let url = self
            .base
            .join(path)
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let mut req = self.http.request(method, url);
        if let Some(body) = body {
            req = req.json(body);
        }
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.json().await?);
        }
        match resp.json::<ApiError>().await {
            Ok(e) => Err(ClientError::Api(e.code, e.message)),
            Err(_) if status == StatusCode::UNAUTHORIZED => Err(ClientError::Api(
                ApiErrorCode::AuthInvalid,
                status.to_string(),
            )),
            Err(_) => Err(ClientError::Network(format!("HTTP {status}"))),
        }
    }
}
