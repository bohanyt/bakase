use super::*;
use crate::{db::Db, tests::app_state};
use aws_lc_rs::{
    rand::SystemRandom,
    rsa::{KeyPair, KeySize},
    signature::{KeyPair as _, RSA_PKCS1_SHA256},
};
use axum::{
    Form, Router,
    body::Body,
    http::{Method, Request},
    response::Html,
    routing::{get, post},
};
use http_body_util::BodyExt;
use jsonwebtoken::{EncodingKey, Header, encode};
use serde_json::{Value, json};
use std::sync::{
    Mutex as Lock, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

const TENANT: Uuid = Uuid::from_u128(100);
const CLIENT: Uuid = Uuid::from_u128(200);
const LEGACY: &str = "lama@example.test";
const ACCESS: &str = "akses@example.test";
const ADMIN: &str = "admin@example.test";
const TEST_PASSWORD: &str = "password-lokal-tes-123";
const SECRET_FIXTURE: &str = "nilai-tiruan";

#[derive(Clone, Default)]
struct Profile {
    email: Option<&'static str>,
    oid: u128,
    changed: Option<(&'static str, Value)>,
    kid: Option<&'static str>,
    alg: Option<Algorithm>,
}

impl Profile {
    fn user(email: &'static str, oid: u128) -> Self {
        Self {
            email: Some(email),
            oid,
            ..Self::default()
        }
    }
}

struct Grant {
    args: HashMap<String, String>,
    profile: Profile,
}

struct Provider {
    url: String,
    jwks_reads: AtomicUsize,
    token_reads: AtomicUsize,
    data: Lock<ProviderData>,
}

#[derive(Default)]
struct ProviderData {
    codes: HashMap<String, Grant>,
    rotated: bool,
    stall: Option<tokio::sync::oneshot::Sender<()>>,
    offline: bool,
}

struct Mock {
    provider: Arc<Provider>,
    task: JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

static KEYS: OnceLock<[KeyPair; 2]> = OnceLock::new();

fn keypair(index: usize) -> &'static KeyPair {
    &KEYS.get_or_init(|| std::array::from_fn(|_| KeyPair::generate(KeySize::Rsa2048).unwrap()))
        [index]
}

impl Provider {
    fn issuer(&self) -> String {
        format!("{}/{TENANT}/v2.0", self.url)
    }

    fn issue(&self, args: &HashMap<String, String>, profile: Profile) -> String {
        let code = auth::new_token();
        self.data.lock().unwrap().codes.insert(
            code.clone(),
            Grant {
                args: args.clone(),
                profile,
            },
        );
        code
    }
}

impl Mock {
    async fn start() -> Self {
        // Pembuatan kunci RSA tidak boleh menghabiskan batas waktu permintaan saat CPU penuh.
        keypair(0);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = Arc::new(Provider {
            url: format!("http://localhost:{}", listener.local_addr().unwrap().port()),
            jwks_reads: AtomicUsize::new(0),
            token_reads: AtomicUsize::new(0),
            data: Lock::new(ProviderData::default()),
        });
        let base = format!("/{TENANT}");
        let app = Router::new()
            .route(&format!("{base}/oauth2/v2.0/authorize"), get(authorize))
            .route(&format!("{base}/oauth2/v2.0/token"), post(exchange))
            .route(&format!("{base}/discovery/v2.0/keys"), get(keys))
            .with_state(provider.clone());
        let task = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
        Self { provider, task }
    }
}

async fn keys(State(provider): State<Arc<Provider>>) -> Response {
    provider.jwks_reads.fetch_add(1, Ordering::Relaxed);
    let data = provider.data.lock().unwrap();
    if data.offline {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let (index, kid) = if data.rotated {
        (1, "rotated")
    } else {
        (0, "current")
    };
    let public = keypair(index).public_key();
    let jwk = json!({
        "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "issuer": provider.issuer(),
        "n": URL_SAFE_NO_PAD.encode(public.modulus().big_endian_without_leading_zero()),
        "e": URL_SAFE_NO_PAD.encode(public.exponent().big_endian_without_leading_zero()),
    });
    Json(json!({ "keys": [jwk] })).into_response()
}

async fn authorize(
    State(provider): State<Arc<Provider>>,
    Query(args): Query<HashMap<String, String>>,
) -> Html<String> {
    let redirect = Url::parse(&args["redirect_uri"]).unwrap();
    assert_eq!(redirect.path(), CALLBACK);
    assert_eq!(args["client_id"], CLIENT.to_string());
    assert_eq!(args["scope"], "openid profile email");
    assert_eq!(args["response_type"], "code");
    assert_eq!(args["code_challenge_method"], "S256");
    let mut html = String::from(
        "<meta charset=utf-8><title>OIDC lokal</title><h1>Penyedia OIDC lokal</h1><ul>",
    );
    for (label, profile) in [
        ("Masuk sebagai karyawan", Profile::user(ACCESS, 2)),
        ("Masuk sebagai admin", Profile::user(ADMIN, 3)),
        (
            "Masuk tanpa akses",
            Profile::user("menunggu@example.test", 4),
        ),
        ("Pilih akun pribadi", Profile::user("other@example.org", 5)),
    ] {
        let code = provider.issue(&args, profile);
        let mut link = redirect.clone();
        link.query_pairs_mut()
            .append_pair("state", &args["state"])
            .append_pair("code", &code);
        html.push_str(&format!(
            "<li><a href='{}'>{label}</a></li>",
            link.as_str().replace('&', "&amp;")
        ));
    }
    let mut cancel = redirect;
    cancel
        .query_pairs_mut()
        .append_pair("state", &args["state"])
        .append_pair("error", "access_denied");
    html.push_str(&format!(
        "<li><a href='{}'>Batalkan</a></li></ul>",
        cancel.as_str().replace('&', "&amp;")
    ));
    Html(html)
}

async fn exchange(
    State(provider): State<Arc<Provider>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    provider.token_reads.fetch_add(1, Ordering::Relaxed);
    let (grant, stall) = {
        let mut data = provider.data.lock().unwrap();
        if data.offline {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let grant = data.codes.remove(&form["code"]);
        (grant, data.stall.take())
    };
    if let Some(stall) = stall {
        let _ = stall.send(());
        std::future::pending::<()>().await;
    }
    let Some(grant) = grant else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    };
    assert_eq!(form["client_secret"], SECRET_FIXTURE);
    assert_eq!(form["client_id"], CLIENT.to_string());
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["redirect_uri"], grant.args["redirect_uri"]);
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
        grant.args["code_challenge"]
    );
    let profile = grant.profile;
    let stamp = now();
    let mut claims = json!({
        "aud": CLIENT, "iss": provider.issuer(), "tid": TENANT,
        "exp": stamp + 600, "nbf": stamp - 60, "iat": stamp - 60,
        "sub": "sub-tiruan", "oid": Uuid::from_u128(profile.oid),
        "acct": 0, "preferred_username": profile.email, "nonce": grant.args["nonce"], "name": "Karyawan Uji",
    });
    if let Some((claim, value)) = profile.changed {
        claims[claim] = value;
    }
    let kid = profile.kid.unwrap_or("current");
    let algorithm = profile.alg.unwrap_or(Algorithm::RS256);
    let mut header = Header::new(algorithm);
    header.kid = Some(if kid == "forged" { "current" } else { kid }.into());
    let id_token = if algorithm == Algorithm::HS256 {
        encode(
            &header,
            &claims,
            &EncodingKey::from_secret(&[42; 32]).unwrap(),
        )
        .unwrap()
    } else {
        let payload = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap()),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        );
        let key = keypair(usize::from(kid == "rotated" || kid == "forged"));
        let mut signature = vec![0; key.public_key().modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            payload.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{payload}.{}", URL_SAFE_NO_PAD.encode(signature))
    };
    Json(json!({"id_token": id_token})).into_response()
}

fn state(mock: &Mock, db: Db, pilot: Vec<String>, redirect_uri: String) -> Arc<AppState> {
    let mut state = Arc::into_inner(app_state()).unwrap_or_else(|| unreachable!());
    state.db = db;
    state.auth.email_domain = "example.test".into();
    state.auth.secure_cookie = true;
    let settings = Settings {
        enabled: true,
        tenant_id: TENANT,
        client_id: CLIENT,
        redirect_uri,
        secret_expires: Some("2027-10-01".parse().unwrap()),
        pilot,
    };
    state.microsoft =
        Some(Microsoft::new(&settings, &mock.provider.url, Some(SECRET_FIXTURE.into())).unwrap());
    state.auth.microsoft = Some(settings);
    Arc::new(state)
}

async fn seed(db: &Db) {
    let hash = auth::hash_password(TEST_PASSWORD).unwrap();
    db.call(move |c| {
        c.execute(
            "INSERT INTO users (id,email,name,password_hash,status,role,department,created_at,approved_at) VALUES
             (51,?1,'Lama',?3,'active','user','Review',0,0),
             (52,?2,'Admin',?3,'active','admin',NULL,0,0),
             (53,'nonaktif@example.test','Nonaktif',?3,'disabled','user','Review',0,0),
             (54,'lain@example.test','Lain',?3,'active','user',NULL,0,0),
             (55,'lokal-pending@example.test','Lokal',?3,'pending','user',NULL,0,NULL)",
            params![LEGACY, ADMIN, hash],
        )?;
        c.execute_batch(
            "INSERT INTO imported_access VALUES
                ('akses@example.test','[\"Operasional\",\"Review\"]'),
                ('kosong@example.test','[]'),
                ('lokal-pending@example.test','[\"Review\"]');
             INSERT INTO conversations VALUES ('riwayat-tes',51,'Tersimpan',0,0);"
        )?;
        Ok(())
    }).await.unwrap();
}

fn request(method: Method, path: &str, body: Option<Value>, cookie: Option<&str>) -> Request<Body> {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        req = req.header(header::COOKIE, cookie);
    }
    let body = if let Some(body) = body {
        req = req.header(header::CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    req.body(body).unwrap()
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> Response {
    app.clone()
        .oneshot(request(method, path, body, cookie))
        .await
        .unwrap()
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn cookie(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.split('=').next() == Some(name))
        .map(str::to_string)
}

fn pair(header: &str) -> &str {
    header.split(';').next().unwrap()
}

fn location(response: &Response) -> &str {
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
    response.headers()[header::LOCATION].to_str().unwrap()
}

struct Attempt {
    args: HashMap<String, String>,
    cookie: String,
}

async fn launch(app: &Router, pilot: bool, panel_tab: bool, return_to: &str) -> Attempt {
    let response = send(
        app,
        Method::POST,
        "/api/auth/microsoft/start",
        Some(json!({
            "pilot": pilot, "panel_tab": panel_tab, "return_to": return_to
        })),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = cookie(&response, BROWSER_COOKIE).unwrap();
    for attr in [
        "Path=/",
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Max-Age=600",
    ] {
        assert!(cookie.split("; ").any(|field| field == attr));
    }
    let body = json_body(response).await;
    let url = Url::parse(body["url"].as_str().unwrap()).unwrap();
    assert_eq!(url.host_str(), Some("localhost"));
    let args: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(args["scope"], "openid profile email");
    assert_eq!(args["response_type"], "code");
    assert_eq!(args["code_challenge_method"], "S256");
    assert_ne!(args["state"], args["nonce"]);
    assert!(!args.contains_key("client_secret"));
    Attempt {
        args,
        cookie: pair(&cookie).into(),
    }
}

impl Attempt {
    fn callback(&self, code: &str) -> String {
        format!("{CALLBACK}?state={}&code={code}", self.args["state"])
    }
}

async fn sign_in(app: &Router, mock: &Mock, profile: Profile, pilot: bool) -> Response {
    let attempt = launch(app, pilot, false, "/").await;
    let code = mock.provider.issue(&attempt.args, profile);
    send(
        app,
        Method::GET,
        &attempt.callback(&code),
        None,
        Some(&attempt.cookie),
    )
    .await
}

async fn me(app: &Router, response: &Response) -> Value {
    let session = cookie(response, auth::SESSION_COOKIE).unwrap();
    assert!(
        session.contains("HttpOnly")
            && session.contains("Secure")
            && session.contains("SameSite=Lax")
    );
    json_body(send(app, Method::GET, "/api/me", None, Some(pair(&session))).await).await
}

#[tokio::test]
async fn legacy_import_pending_and_admin_keep_correct_access() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let app = crate::router(state(&mock, db.clone(), vec![], REGISTERED_REDIRECT.into()));

    let legacy = sign_in(&app, &mock, Profile::user(LEGACY, 11), false).await;
    assert_eq!(location(&legacy), "/");
    let legacy_user = me(&app, &legacy).await;
    assert_eq!(legacy_user["id"], 51);
    assert_eq!(legacy_user["departments"], json!(["Review"]));
    let session = cookie(&legacy, auth::SESSION_COOKIE).unwrap();
    let history = json_body(
        send(
            &app,
            Method::GET,
            "/api/conversations",
            None,
            Some(pair(&session)),
        )
        .await,
    )
    .await;
    assert!(history.to_string().contains("riwayat-tes"));

    let newly = sign_in(&app, &mock, Profile::user(ACCESS, 22), false).await;
    let user = me(&app, &newly).await;
    assert_eq!(user["departments"], json!(["Operasional", "Review"]));
    assert_eq!(user["role"], "user");
    let admin = sign_in(&app, &mock, Profile::user(ADMIN, 33), false).await;
    assert_eq!(me(&app, &admin).await["role"], "admin");
    let local_pending = sign_in(
        &app,
        &mock,
        Profile::user("lokal-pending@example.test", 35),
        false,
    )
    .await;
    assert_eq!(me(&app, &local_pending).await["id"], 55);

    // #139: akun kantor tanpa departemen dari impor ARCHIVE langsung aktif dengan akses Umum, kecuali
    // alamatnya dilarang di ekspor terakhir.
    db.call(|c| {
        c.execute(
            "INSERT INTO imported_access_bans VALUES ('dilarang@example.test')",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    for (email, oid, status, departments) in [
        ("menunggu@example.test", 44, "active", "[\"Umum\"]"),
        ("kosong@example.test", 55, "active", "[\"Umum\"]"),
        ("dilarang@example.test", 66, "pending", "[]"),
    ] {
        let signed = sign_in(&app, &mock, Profile::user(email, oid), false).await;
        if status == "active" {
            assert_eq!(location(&signed), "/");
            assert_eq!(me(&app, &signed).await["departments"], json!(["Umum"]));
        } else {
            assert_eq!(location(&signed), "/?ms=access");
            assert!(cookie(&signed, auth::SESSION_COOKIE).is_none());
        }
        db.call(move |db| {
            let actual = db.query_row(
                "SELECT status,departments,role FROM users_with_access WHERE email=?1",
                [email],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )?;
            assert_eq!(actual, (status.into(), departments.into(), "user".into()));
            Ok(())
        })
        .await
        .unwrap();
    }

    db.call(|db| {
        let id = db.query_row(
            "SELECT id FROM users WHERE email = 'menunggu@example.test'",
            [],
            |r| r.get(0),
        )?;
        let tx = db.transaction()?;
        access::replace(&tx, id, &["Review".into()])?;
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
    let approved = sign_in(
        &app,
        &mock,
        Profile::user("menunggu@example.test", 44),
        false,
    )
    .await;
    assert_eq!(me(&app, &approved).await["departments"], json!(["Review"]));

    let changed_address = sign_in(&app, &mock, Profile::user("lain@example.test", 11), false).await;
    let renamed = me(&app, &changed_address).await;
    assert_eq!(renamed["id"], 51);
    assert_eq!(renamed["email"], LEGACY);
    let conflict = sign_in(&app, &mock, Profile::user(LEGACY, 99), false).await;
    assert_eq!(location(&conflict), "/?ms=linked");
    for (email, oid) in [("nonaktif@example.test", 77), (LEGACY, 11)] {
        db.call(|c| {
            c.execute("UPDATE users SET status='disabled' WHERE id=51", [])?;
            Ok(())
        })
        .await
        .unwrap();
        let disabled = sign_in(&app, &mock, Profile::user(email, oid), false).await;
        assert_eq!(location(&disabled), "/?ms=disabled");
    }
    assert_eq!(mock.provider.jwks_reads.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn rejects_invalid_claims_signatures_guests_and_missing_work_address() {
    let mock = Mock::start().await;
    let app = crate::router(state(
        &mock,
        Db::open_in_memory().unwrap(),
        vec![],
        REGISTERED_REDIRECT.into(),
    ));
    let valid = Profile::user(LEGACY, 11);
    for (name, value, expected) in [
        ("iss", json!("https://issuer.example.test/v2.0"), "invalid"),
        ("aud", json!(Uuid::from_u128(201)), "invalid"),
        ("tid", json!(Uuid::from_u128(101)), "invalid"),
        ("nonce", json!("tidak-cocok"), "invalid"),
        ("exp", json!(now() - 120), "invalid"),
        ("nbf", json!(now() + 120), "invalid"),
        ("acct", json!(1), "wrong"),
        ("acct", json!("1"), "wrong"),
        (
            "preferred_username",
            json!("tamu_example.org#EXT#@example.test"),
            "wrong",
        ),
        ("preferred_username", json!("palsu@example.org"), "wrong"),
        ("preferred_username", json!(""), "wrong"),
        ("preferred_username", Value::Null, "wrong"),
    ] {
        let response = sign_in(
            &app,
            &mock,
            Profile {
                changed: Some((name, value)),
                ..valid.clone()
            },
            false,
        )
        .await;
        assert_eq!(location(&response), format!("/?ms={expected}"), "{name}");
        assert!(cookie(&response, auth::SESSION_COOKIE).is_none());
    }
    for profile in [
        Profile {
            alg: Some(Algorithm::HS256),
            ..valid.clone()
        },
        Profile {
            kid: Some("forged"),
            ..valid.clone()
        },
        Profile {
            kid: Some("unknown"),
            ..valid
        },
    ] {
        let response = sign_in(&app, &mock, profile, false).await;
        assert_eq!(location(&response), "/?ms=invalid");
        assert!(cookie(&response, auth::SESSION_COOKIE).is_none());
    }
    assert_eq!(mock.provider.jwks_reads.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn rotates_keys_and_enforces_browser_binding_state_and_code_replay() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let app = crate::router(state(&mock, db.clone(), vec![], REGISTERED_REDIRECT.into()));
    let attempt = launch(&app, false, false, "/?asal=tes#arsip").await;
    mock.provider.data.lock().unwrap().rotated = true;
    let code = mock.provider.issue(
        &attempt.args,
        Profile {
            kid: Some("rotated"),
            ..Profile::user(LEGACY, 11)
        },
    );
    let path = attempt.callback(&code);

    let wrong_state = format!("{CALLBACK}?state=salah&code={code}");
    for (path, jar) in [
        (path.as_str(), None),
        (path.as_str(), Some("__Host-app_ms=salah")),
        (wrong_state.as_str(), Some(attempt.cookie.as_str())),
    ] {
        let res = send(&app, Method::GET, path, None, jar).await;
        assert_eq!(location(&res), "/?ms=expired");
    }
    assert_eq!(mock.provider.token_reads.load(Ordering::Relaxed), 0);
    let (first, second) = tokio::join!(
        send(&app, Method::GET, &path, None, Some(&attempt.cookie)),
        send(&app, Method::GET, &path, None, Some(&attempt.cookie)),
    );
    let redirects = [location(&first), location(&second)];
    assert!(redirects.contains(&"/?asal=tes#arsip") && redirects.contains(&"/?ms=expired"));
    assert_eq!(mock.provider.jwks_reads.load(Ordering::Relaxed), 2);
    assert_eq!(mock.provider.token_reads.load(Ordering::Relaxed), 1);

    let next = launch(&app, false, false, "/").await;
    let replay = send(
        &app,
        Method::GET,
        &next.callback(&code),
        None,
        Some(&next.cookie),
    )
    .await;
    assert_eq!(location(&replay), "/?ms=invalid");
    assert!(cookie(&replay, auth::SESSION_COOKIE).is_none());
}

#[tokio::test]
async fn expiration_cancellation_and_provider_outage_have_clear_results() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    let app = crate::router(state(&mock, db.clone(), vec![], REGISTERED_REDIRECT.into()));

    let expired = launch(&app, false, false, "/").await;
    db.call(|c| {
        c.execute("UPDATE microsoft_flows SET expires_at=0", [])?;
        Ok(())
    })
    .await
    .unwrap();
    let response = send(
        &app,
        Method::GET,
        &expired.callback("expired"),
        None,
        Some(&expired.cookie),
    )
    .await;
    assert_eq!(location(&response), "/?ms=expired");

    let canceled = launch(&app, false, false, "/").await;
    let path = format!(
        "{CALLBACK}?state={}&error=access_denied",
        canceled.args["state"]
    );
    let response = send(&app, Method::GET, &path, None, Some(&canceled.cookie)).await;
    assert_eq!(location(&response), "/?ms=canceled");
    let reused = send(&app, Method::GET, &path, None, Some(&canceled.cookie)).await;
    assert_eq!(location(&reused), "/?ms=expired");
    assert_eq!(mock.provider.token_reads.load(Ordering::Relaxed), 0);

    mock.provider.data.lock().unwrap().offline = true;
    let response = sign_in(&app, &mock, Profile::user(LEGACY, 11), false).await;
    assert_eq!(location(&response), "/?ms=unavailable");
    let fresh = crate::router(state(&mock, db, vec![], REGISTERED_REDIRECT.into()));
    let response = send(
        &fresh,
        Method::POST,
        "/api/auth/microsoft/start",
        Some(json!({})),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn stalled_exchange_reaches_deadline() {
    let mock = Mock::start().await;
    let app = crate::router(state(
        &mock,
        Db::open_in_memory().unwrap(),
        vec![],
        REGISTERED_REDIRECT.into(),
    ));
    let attempt = launch(&app, false, false, "/").await;
    let code = mock
        .provider
        .issue(&attempt.args, Profile::user(ACCESS, 22));
    let (waiting, arrived) = tokio::sync::oneshot::channel();
    mock.provider.data.lock().unwrap().stall = Some(waiting);
    let path = attempt.callback(&code);
    let response =
        tokio::spawn(
            async move { send(&app, Method::GET, &path, None, Some(&attempt.cookie)).await },
        );
    arrived.await.unwrap();
    tokio::time::pause();
    tokio::time::advance(TIMEOUT).await;
    assert_eq!(location(&response.await.unwrap()), "/?ms=unavailable");
}

#[tokio::test]
async fn mail_address_and_sign_in_name_both_identify_the_account() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let mail = |value: &str, profile: Profile| Profile {
        changed: Some(("email", json!(value))),
        ..profile
    };
    let pilot = crate::router(state(
        &mock,
        db.clone(),
        vec![ADMIN.into()],
        REGISTERED_REDIRECT.into(),
    ));
    let full = crate::router(state(&mock, db.clone(), vec![], REGISTERED_REDIRECT.into()));

    // Nama masuk berbeda; alamat email ada di daftar uji dan milik akun admin.
    let admin = mail(ADMIN, Profile::user("nama.masuk@example.test", 81));
    let response = sign_in(&pilot, &mock, admin, true).await;
    let user = me(&pilot, &response).await;
    assert_eq!((&user["id"], &user["role"]), (&json!(52), &json!("admin")));

    // Alamat email luar diabaikan, dan tidak menggantikan nama masuk kantor.
    let outside = mail("orang@example.org", Profile::user(LEGACY, 82));
    let response = sign_in(&full, &mock, outside, false).await;
    assert_eq!(me(&full, &response).await["id"], 51);

    // Akun baru memakai alamat email dan akses impornya.
    let new = mail(ACCESS, Profile::user("baru.masuk@example.test", 83));
    let response = sign_in(&full, &mock, new, false).await;
    let user = me(&full, &response).await;
    assert_eq!(user["email"], ACCESS);
    assert_eq!(user["departments"], json!(["Operasional", "Review"]));

    // Dua akun berbeda untuk satu orang tidak ditebak.
    let both = mail(
        "lain@example.test",
        Profile::user("lokal-pending@example.test", 84),
    );
    let response = sign_in(&full, &mock, both, false).await;
    assert_eq!(location(&response), "/?ms=linked");
    db.call(|c| {
        let linked: i64 = c.query_row(
            "SELECT count(*) FROM microsoft_identities WHERE user_id IN (54, 55)",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(linked, 0);
        Ok(())
    })
    .await
    .unwrap();

    // Alamat email kantor saja tidak cukup kalau nama masuknya bukan alamat kantor.
    let guest = mail(
        LEGACY,
        Profile::user("tamu_example.org#EXT#@example.test", 85),
    );
    let response = sign_in(&full, &mock, guest, false).await;
    assert_eq!(location(&response), "/?ms=wrong");
}

#[tokio::test]
async fn imported_access_follows_the_mail_address_on_every_login() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let app = crate::router(state(&mock, db.clone(), vec![], REGISTERED_REDIRECT.into()));
    let person = |mail: &str, sign_in: &'static str, oid| Profile {
        changed: Some(("email", json!(mail))),
        ..Profile::user(sign_in, oid)
    };
    let import = async |email: &'static str, departments: &'static str| {
        db.call(move |c| {
            c.execute(
                "INSERT INTO imported_access VALUES (?1, ?2)",
                params![email, departments],
            )?;
            Ok(())
        })
        .await
        .unwrap()
    };
    let rows = async || {
        db.call(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM imported_access WHERE email IN
                 ('surat@example.test','masuk@example.test','surat2@example.test','lain@example.test')",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap()
    };

    // Akun lama tercatat dengan nama masuk, daftar akses memakai alamat email.
    import("surat2@example.test", "[\"Review\"]").await;
    let response = sign_in(
        &app,
        &mock,
        person("surat2@example.test", "lain@example.test", 91),
        false,
    )
    .await;
    let user = me(&app, &response).await;
    assert_eq!(
        (&user["id"], &user["departments"]),
        (&json!(54), &json!(["Review"]))
    );

    // Impor berikutnya lewat alamat email berlaku saat masuk lagi, termasuk pencabutan. Baris
    // lama untuk nama masuk tidak mengembalikan akses.
    for (email, departments, expected) in [
        ("surat2@example.test", "[]", json!([])),
        ("lain@example.test", "[\"Review\"]", json!([])),
        (
            "surat2@example.test",
            "[\"Operasional\"]",
            json!(["Operasional"]),
        ),
    ] {
        import(email, departments).await;
        let response = sign_in(
            &app,
            &mock,
            person("surat2@example.test", "lain@example.test", 91),
            false,
        )
        .await;
        assert_eq!(
            me(&app, &response).await["departments"],
            expected,
            "{email}"
        );
    }

    // Token tanpa alamat email tidak membuat baris lama untuk nama masuk berlaku.
    db.call(|c| {
        c.execute(
            "UPDATE users SET email = 'surat2@example.test', department = NULL WHERE id = 54",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    import("lain@example.test", "[\"Review\"]").await;
    for _ in 0..2 {
        let response = sign_in(&app, &mock, Profile::user("lain@example.test", 91), false).await;
        assert_eq!(me(&app, &response).await["departments"], json!([]));
    }

    // Impor kosong di alamat email tidak memberi departemen impor; akun kantor Menunggu itu tetap
    // aktif dengan akses Umum, juga pada login berikutnya.
    db.call(|c| {
        c.execute_batch(
            "INSERT INTO users (id,email,name,password_hash,status,role,department,created_at)
             VALUES (57,'masuk@example.test','Lokal','x','pending','user','Review',0);",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    import("surat@example.test", "[]").await;
    for _ in 0..2 {
        let response = sign_in(
            &app,
            &mock,
            person("surat@example.test", "masuk@example.test", 92),
            false,
        )
        .await;
        assert_eq!(location(&response), "/");
        assert_eq!(me(&app, &response).await["departments"], json!(["Umum"]));
    }
    assert_eq!(rows().await, 0);

    // Akun baru yang alamat emailnya tidak ada di daftar memakai baris nama masuk.
    import("masuk3@example.test", "[\"Review\"]").await;
    let response = sign_in(
        &app,
        &mock,
        person("surat3@example.test", "masuk3@example.test", 94),
        false,
    )
    .await;
    let user = me(&app, &response).await;
    assert_eq!(
        (&user["email"], &user["departments"]),
        (&json!("surat3@example.test"), &json!(["Review"]))
    );

    // Akun admin tidak diwarisi lewat alamat email saja.
    let response = sign_in(
        &app,
        &mock,
        person(ADMIN, "orang.lain@example.test", 93),
        false,
    )
    .await;
    assert_eq!(location(&response), "/?ms=linked");
}

#[tokio::test]
async fn refreshes_verified_aliases_without_stealing_another_accounts_address() {
    let db = Db::open_in_memory().unwrap();
    db.call(|c| {
        c.execute_batch(
            "INSERT INTO users (id,email,name,password_hash,status,department,created_at) VALUES
                (1,'utama@example.test','A','','active','Review',0),
                (2,'milik-akun@example.test','B','','active','Review',0),
                (3,'lain@example.test','C','','active',NULL,0);
             INSERT INTO microsoft_identities VALUES ('t','a',1), ('t','b',2), ('t','c',3);
             INSERT INTO microsoft_addresses VALUES
                ('lama@example.test',1,1), ('milik-alias@example.test',2,1);
             INSERT INTO imported_access VALUES
                ('milik-akun@example.test','[]'), ('milik-alias@example.test','[]');",
        )?;
        let tx = c.transaction()?;
        let claims = [
            "milik-akun@example.test".to_owned(),
            "milik-alias@example.test".to_owned(),
        ];
        assert_eq!(
            link_account(&tx, claims.first(), &claims, "A", "t", "a", &[], &[])?,
            Ok(1)
        );
        let order = |tx: &Transaction<'_>| -> rusqlite::Result<Vec<(String, i64, bool)>> {
            tx.prepare("SELECT email,user_id,is_mail FROM microsoft_addresses ORDER BY email")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        };
        assert_eq!(
            order(&tx)?,
            [("milik-alias@example.test".to_owned(), 2, true)]
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM imported_access", [], |r| r
                .get::<_, i64>(0))?,
            2
        );
        assert_eq!(
            tx.query_row("SELECT department FROM users WHERE id=1", [], |r| r
                .get::<_, String>(0))?,
            "Review",
        );
        let claims = [
            "baru@example.test".to_owned(),
            "masuk@example.test".to_owned(),
        ];
        assert_eq!(
            link_account(&tx, claims.first(), &claims, "A", "t", "a", &[], &[])?,
            Ok(1)
        );
        assert_eq!(
            order(&tx)?,
            [
                ("baru@example.test".to_owned(), 1, true),
                ("masuk@example.test".to_owned(), 1, false),
                ("milik-alias@example.test".to_owned(), 2, true),
            ]
        );
        tx.execute("UPDATE users SET email='masuk@example.test' WHERE id=3", [])?;
        assert_eq!(order(&tx)?.len(), 2);
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn staged_mail_does_not_override_a_ban_at_the_other_address() {
    let db = Db::open_in_memory().unwrap();
    db.call(|c| {
        c.execute_batch(
            "INSERT INTO users (id,email,name,password_hash,status,department,created_at)
                VALUES (1,'masuk@example.test','A','','active',NULL,0);
             INSERT INTO imported_access VALUES ('surat@example.test','[\"Review\"]');
             INSERT INTO imported_access_bans VALUES ('masuk@example.test');",
        )?;
        let tx = c.transaction()?;
        let claims = [
            "surat@example.test".to_owned(),
            "masuk@example.test".to_owned(),
        ];
        assert_eq!(
            link_account(&tx, claims.first(), &claims, "A", "t", "a", &[], &[])?,
            Ok(1)
        );
        assert_eq!(
            tx.query_row("SELECT department FROM users WHERE id=1", [], |r| r
                .get::<_, Option<String>>(0))?,
            None
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM imported_access", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_later_complete_import_revokes_and_disposes_of_stale_mail_before_login() {
    use std::fs;

    let dir = std::env::temp_dir().join(format!("app-snapshot-{}", crate::files::new_id()));
    let db = Db::open(&dir).unwrap();
    let config = dir.join("uji.toml");
    fs::write(
        &config,
        r#"data_dir = "."
        [azure]
        endpoint = "https://example.test"
        deployment = "uji"
        [auth]
        email_domain = "example.test"
        departments = ["Review"]
    "#,
    )
    .unwrap();
    let export = dir.join("uji.json");
    let run = async |body: &str, revoke: bool| {
        fs::write(&export, body).unwrap();
        let mut args = vec![
            export.to_string_lossy().into_owned(),
            "--peta".into(),
            "Q=Review".into(),
            "--config".into(),
            config.to_string_lossy().into_owned(),
            "--terapkan".into(),
        ];
        if revoke {
            args.extend(["--cabut-yang-absen".into(), "--konfirmasi-impor".into()]);
        }
        crate::import_access::cli(&args).await.unwrap();
    };
    db.call(|c| {
        c.execute_batch(
            "INSERT INTO users (id,email,name,password_hash,status,department,created_at)
                VALUES (1,'masuk@example.test','A','','active','Review',0);
             INSERT INTO microsoft_identities VALUES ('t','a',1);
             INSERT INTO sessions VALUES ('uji',1,0,100);",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    run(
        r#"{"users":[{"email":"surat@example.test","apps":["Q"],"banned":false}]}"#,
        false,
    )
    .await;
    run(
        r#"{"users":[{"email":"lain@example.test","apps":["Q"],"banned":false}]}"#,
        true,
    )
    .await;
    db.call(|c| {
        let tx = c.transaction()?;
        let claims = [
            "surat@example.test".to_owned(),
            "masuk@example.test".to_owned(),
        ];
        assert_eq!(
            link_account(&tx, claims.first(), &claims, "A", "t", "a", &[], &[])?,
            Ok(1)
        );
        assert_eq!(
            tx.query_row("SELECT department FROM users WHERE id=1", [], |r| r
                .get::<_, Option<String>>(0))?,
            None
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM sessions WHERE user_id=1", [], |r| r
                .get::<_, i64>(
                0
            ))?,
            0
        );
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
    drop(db);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn optional_member_claim_is_not_required_for_work_addresses() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let app = crate::router(state(&mock, db, vec![], REGISTERED_REDIRECT.into()));
    for (email, expected) in [(LEGACY, "/"), ("tamu@example.org", "/?ms=wrong")] {
        let response = sign_in(
            &app,
            &mock,
            Profile {
                changed: Some(("acct", Value::Null)),
                ..Profile::user(email, 71)
            },
            false,
        )
        .await;
        assert_eq!(location(&response), expected, "{email}");
    }
}

#[tokio::test]
async fn unverified_local_password_never_reaches_the_linked_account() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    db.call(|c| {
        c.execute(
            "INSERT INTO users (id,email,name,password_hash,status,role,created_at)
             SELECT 56,'lokal-kosong@example.test','Lokal',password_hash,'pending','user',0
             FROM users WHERE id = 55",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let pilot = vec![
        LEGACY.to_string(),
        ADMIN.to_string(),
        "lokal-pending@example.test".into(),
        "lokal-kosong@example.test".into(),
    ];
    let app = crate::router(state(&mock, db.clone(), pilot, REGISTERED_REDIRECT.into()));
    for (email, oid) in [
        (LEGACY, 61),
        (ADMIN, 62),
        ("lokal-pending@example.test", 63),
        ("lokal-kosong@example.test", 64),
    ] {
        sign_in(&app, &mock, Profile::user(email, oid), true).await;
    }
    // Admin menyetujui belakangan; password lama pendaftar tetap tidak berlaku.
    db.call(|c| {
        c.execute(
            "UPDATE users SET status = 'active', approved_at = 1 WHERE id = 56",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    for (email, expected) in [
        (LEGACY, StatusCode::OK),
        (ADMIN, StatusCode::OK),
        ("lokal-pending@example.test", StatusCode::UNAUTHORIZED),
        ("lokal-kosong@example.test", StatusCode::UNAUTHORIZED),
    ] {
        let login = send(
            &app,
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": TEST_PASSWORD })),
            None,
        )
        .await;
        assert_eq!(login.status(), expected, "{email}");
    }
}

#[tokio::test]
async fn pilot_preserves_password_and_registration_full_mode_limits_both() {
    let mock = Mock::start().await;
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let pilot = crate::router(state(
        &mock,
        db.clone(),
        vec![LEGACY.into()],
        REGISTERED_REDIRECT.into(),
    ));
    let full = crate::router(state(&mock, db, vec![], REGISTERED_REDIRECT.into()));
    let off = crate::router(app_state());

    for (app, mode, register_status, local_status) in [
        (&pilot, json!("pilot"), StatusCode::CREATED, StatusCode::OK),
        (
            &full,
            json!("full"),
            StatusCode::FORBIDDEN,
            StatusCode::UNAUTHORIZED,
        ),
        (&off, Value::Null, StatusCode::CREATED, StatusCode::CREATED),
    ] {
        let options =
            json_body(send(app, Method::GET, "/api/auth/options", None, None).await).await;
        assert_eq!(options["microsoft"], mode);
        assert!(!options.to_string().contains(LEGACY));
        if mode.is_null() {
            assert!(options.get("microsoft").is_none());
        }
        let email = if mode.is_null() {
            "baru@example.com"
        } else {
            "baru@example.test"
        };
        let registration = send(
            app,
            Method::POST,
            "/api/auth/register",
            Some(json!({
                "email": email, "name": "Karyawan Tes", "password": TEST_PASSWORD,
            })),
            None,
        )
        .await;
        assert_eq!(registration.status(), register_status);
        if !mode.is_null() {
            let password = send(
                app,
                Method::POST,
                "/api/auth/login",
                Some(json!({
                    "email": LEGACY, "password": TEST_PASSWORD,
                })),
                None,
            )
            .await;
            assert_eq!(password.status(), local_status);
            let admin = send(
                app,
                Method::POST,
                "/api/auth/login",
                Some(json!({
                    "email": ADMIN, "password": TEST_PASSWORD,
                })),
                None,
            )
            .await;
            assert_eq!(admin.status(), StatusCode::OK);
        }
        let start = send(
            app,
            Method::POST,
            "/api/auth/microsoft/start",
            Some(json!({})),
            None,
        )
        .await;
        assert_eq!(
            start.status(),
            if mode == "full" {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
    }
    let allowed = sign_in(&pilot, &mock, Profile::user(LEGACY, 11), true).await;
    assert_eq!(me(&pilot, &allowed).await["id"], 51);
    let outside = sign_in(&pilot, &mock, Profile::user(ADMIN, 33), true).await;
    assert_eq!(location(&outside), "/?ms=pilot&microsoft=pilot");
    let retired = send(
        &off,
        Method::GET,
        &format!("{CALLBACK}?state=lama"),
        None,
        None,
    )
    .await;
    assert_eq!(location(&retired), "/?ms=expired");
}

#[tokio::test]
async fn flow_and_sessions_cross_slots_and_legacy_sql_still_works() {
    let mock = Mock::start().await;
    let folder = std::env::temp_dir().join(format!("app-ms-test-{}", auth::new_token()));
    let a = Db::open(&folder).unwrap();
    let b = Db::open(&folder).unwrap();
    seed(&a).await;
    let start_slot = crate::router(state(&mock, a, vec![], REGISTERED_REDIRECT.into()));
    let end_slot = crate::router(state(&mock, b.clone(), vec![], REGISTERED_REDIRECT.into()));
    let attempt = launch(&start_slot, false, true, "//example.test").await;
    let code = mock
        .provider
        .issue(&attempt.args, Profile::user(ACCESS, 22));
    let res = send(
        &end_slot,
        Method::GET,
        &attempt.callback(&code),
        None,
        Some(&attempt.cookie),
    )
    .await;
    assert_eq!(location(&res), "/?ms_tab=1&ms=done");
    let user = me(&start_slot, &res).await;
    let hash = auth::token_hash(
        pair(&cookie(&res, auth::SESSION_COOKIE).unwrap())
            .split_once('=')
            .unwrap()
            .1,
    );
    {
        let old = rusqlite::Connection::open(folder.join("rag-hub.sqlite")).unwrap();
        // 204d776 startup keeps user_version=10 and uses these session and access tables.
        let version: i64 = old
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 10);
        let legacy: (i64, String) = old.query_row(
            "SELECT u.id, u.departments FROM sessions s JOIN users_with_access u ON u.id = s.user_id
             WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND u.status = 'active'",
            params![hash, now()], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(legacy.0, user["id"].as_i64().unwrap());
        assert_eq!(legacy.1, r#"["Operasional","Review"]"#);
        old.execute("DELETE FROM sessions WHERE token_hash = ?1", [hash])
            .unwrap();
    }
    let session = cookie(&res, auth::SESSION_COOKIE).unwrap();
    assert_eq!(
        send(
            &end_slot,
            Method::GET,
            "/api/me",
            None,
            Some(pair(&session))
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    drop((start_slot, end_slot, b));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn redirect_config_and_sixty_day_expiry_window() {
    let mut settings = Settings {
        enabled: true,
        tenant_id: TENANT,
        client_id: CLIENT,
        redirect_uri: REGISTERED_REDIRECT.into(),
        secret_expires: Some("2027-10-01".parse().unwrap()),
        pilot: vec!["LAMA@EXAMPLE.TEST".into()],
    };
    assert!(settings.validate("example.test", false).is_err());
    settings.validate("example.test", true).unwrap();
    assert_eq!(settings.pilot, [LEGACY]);
    let date = settings.expiry_date().unwrap();
    assert_eq!(
        settings.expiry_warning(date - time::Duration::days(61)),
        None
    );
    assert_eq!(
        settings.expiry_warning(date - time::Duration::days(60)),
        Some(60)
    );
    assert_eq!(
        settings.expiry_warning(date - time::Duration::days(14)),
        Some(14)
    );
    assert_eq!(
        settings.expiry_warning(date + time::Duration::days(1)),
        Some(-1)
    );
    settings.redirect_uri.push('/');
    assert!(settings.validate("example.test", true).is_err());
    for unsafe_path in [
        "//external.test",
        "/\\external.test",
        "/%2fexternal.test",
        "https://external.test",
        "/api/me",
        "/../login",
        "/x\r\nLocation:x",
    ] {
        assert_eq!(safe_return_to(unsafe_path), "/");
    }
    assert_eq!(safe_return_to("/?asal=tes#chat"), "/?asal=tes#chat");
}

#[tokio::test]
#[ignore = "server loopback untuk pemeriksaan UI manual, hentikan dengan Ctrl+C"]
async fn browser_fixture() {
    let Ok(port) = std::env::var("APP_TEST_OIDC_BROWSER_PORT") else {
        return;
    };
    let port: u16 = port.parse().unwrap();
    let mock = Mock::start().await;
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let db = Db::open_in_memory().unwrap();
    seed(&db).await;
    let state = state(
        &mock,
        db,
        vec![],
        format!("http://127.0.0.1:{port}{CALLBACK}"),
    );
    let mut state = Arc::into_inner(state).unwrap_or_else(|| unreachable!());
    state.guest.enabled = true;
    state.support_whatsapp = None;
    if std::env::var_os("APP_TEST_OIDC_BROWSER_OFF").is_some() {
        state.auth.microsoft = None;
        state.microsoft = None;
    }
    if let Some(settings) = &mut state.auth.microsoft {
        settings.secret_expires = Some(
            (time::OffsetDateTime::now_utc().date() + time::Duration::days(14))
                .to_string()
                .parse()
                .unwrap(),
        );
    }
    let app = crate::router(Arc::new(state))
        .route("/__panel", get(|| async {
            Html(r#"<!doctype html><title>Panel uji</title><style>body{margin:0} iframe{border:0;width:100%;height:100dvh;display:block}</style><iframe title="Bakase" src="/?panel=1"></iframe>"#)
        }))
        .fallback_service(tower_http::services::ServeDir::new("../web/dist"));
    println!("Fixture OIDC lokal siap pada port {port}.");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.unwrap();
        })
        .await
        .unwrap();
}
