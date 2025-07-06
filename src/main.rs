use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::web::Data;
use actix_web::web::PayloadConfig;
use actix_web::{
    web, App, Error, HttpMessage, HttpRequest, HttpResponse, HttpServer, Responder, Result,
};
use clap::Parser;
use pulldown_cmark::Parser as MdParser;
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader as XmlReader;
use rand::prelude::IndexedRandom;
use rand::seq::SliceRandom;
use rand::thread_rng;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::Instant;

// ASCII art for /burn endpoint
const FIRE_ART: &str = r#"
⠀⠀⠀⠀⠀⠀⢱⣆⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠈⣿⣷⡀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⢸⣿⣿⣷⣧⠀⠀⠀
⠀⠀⠀⠀⡀⢠⣿⡟⣿⣿⣿⡇⠀⠀
⠀⠀⠀⠀⣳⣼⣿⡏⢸⣿⣿⣿⢀⠀
⠀⠀⠀⣰⣿⣿⡿⠁⢸⣿⣿⡟⣼⡆
⢰⢀⣾⣿⣿⠟⠀⠀⣾⢿⣿⣿⣿⣿
⢸⣿⣿⣿⡏⠀⠀⠀⠃⠸⣿⣿⣿⡿
⢳⣿⣿⣿⠀⠀⠀⠀⠀⠀⢹⣿⡿⡁
⠀⠹⣿⣿⡄⠀⠀⠀⠀⠀⢠⣿⡞⠁
⠀⠀⠈⠛⢿⣄⠀⠀⠀⣠⠞⠋⠀⠀
⠀⠀⠀⠀⠀⠀⠉⠀⠀⠀⠀⠀⠀⠀
------------------
 BURNED TO ASHES!
"#;

// Response for /pulverize endpoint
#[derive(Serialize)]
struct PulverizeResponse {
    status: &'static str,
    message: &'static str,
    runtime_us: u128,
}

// Response for /shred endpoint
#[derive(Serialize)]
struct ShredResponse {
    status: &'static str,
    log: Vec<&'static str>,
    runtime_us: u128,
}

#[derive(Serialize)]
struct BurnResponse {
    status: &'static str,
    message: &'static str,
    fire: &'static str,
    runtime_us: u128,
}

#[derive(Serialize)]
struct ValidationReport {
    is_json: bool,
    is_xml: bool,
    is_markdown: bool,
    details: Vec<String>,
    runtime_us: u128,
}

// List of all endpoints to track
const ENDPOINTS: &[&str] = &[
    "pulverize",
    "blackhole",
    "shred",
    "burn",
    "validate-before-destroy",
];

// CLI arguments
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the SQLite database file
    #[arg(long, default_value = "/tmp/payload-pulverizer.db")]
    db_path: String,
}

// Update init_db to take a path
fn init_db(db_path: &str) -> Connection {
    let conn = Connection::open(db_path).expect("Failed to open database");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS endpoint_stats_raw (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            endpoint TEXT NOT NULL,
            payload_size INTEGER NOT NULL,
            runtime_us INTEGER NOT NULL,
            ts DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )
    .expect("Failed to create stats table");
    conn
}

// Insert a row for every request
fn record_stat(conn: &Mutex<Connection>, endpoint: &str, payload_size: usize, runtime_us: u128) {
    let _ = conn.lock().unwrap().execute(
        "INSERT INTO endpoint_stats_raw (endpoint, payload_size, runtime_us) VALUES (?1, ?2, ?3)",
        params![endpoint, payload_size as i64, runtime_us as i64],
    );
}

// Update StatsEntry and StatsResponse to match the new aggregation
#[derive(Serialize)]
struct StatsEntry {
    endpoint: String,
    count: i64,
    total_bytes: i64,
    total_runtime_us: i64,
    avg_payload_size: f64,
    avg_runtime_us: f64,
}

#[derive(Serialize)]
struct StatsResponse {
    stats: Vec<StatsEntry>,
}

// Add a list of log message sequences for the shredder
const SHREDDER_LOGS: &[&[&str]] = &[
    &[
        "Feeding payload into industrial-grade data shredder...",
        "Shredding...",
        "Payload particles irreversibly scattered in cyberspace dust."
    ],
    &[
        "Payload enters the shredder. It never stood a chance.",
        "Blades spinning at ludicrous speed...",
        "Payload reduced to confetti. Hope you didn't need that."
    ],
    &[
        "Payload, meet Mr. Shredder.",
        "Mr. Shredder, do your thing.",
        "Payload is now a fine digital powder."
    ],
    &[
        "Initiating payload obliteration protocol...",
        "Warning: No undo button detected.",
        "Payload is now a memory. A very faint one."
    ],
    &[
        "Payload bravely volunteers for shredding.",
        "Shredder: 'I was born for this.'",
        "Payload: 'Tell my bits I love them.'"
    ],
    &[
        "Payload enters the vortex of doom...",
        "Shredder cackles maniacally.",
        "Payload is now existentially challenged."
    ],
    &[
        "Payload: 'I regret nothing!'",
        "Shredder: 'You will.'",
        "Payload is now a cautionary tale."
    ],
    &[
        "Payload is serenaded by the whirring of blades...",
        "Shredder: 'This is my jam.'",
        "Payload is now a remix of its former self."
    ],
    &[
        "Payload enters the shredder's lair.",
        "Shredder: 'Another one for the collection.'",
        "Payload is now a collectible dust bunny."
    ],
    &[
        "Payload: 'Is this going to hurt?'",
        "Shredder: 'Only for a microsecond.'",
        "Payload is now at peace."
    ],
    &[
        "Payload is weighed, measured, and found... shreddable.",
        "Shredder: 'I love my job.'",
        "Payload is now a statistic."
    ],
    &[
        "Payload is greeted by the Shredder's motivational poster: 'You miss 100% of the bits you don't shred.'",
        "Shredder warms up with a few practice spins.",
        "Payload is now a motivational example for others.",
        "Shredder: 'Next!'"
    ],
    &[
        "Payload: 'I was told there would be snacks.'",
        "Shredder: 'You are the snack.'",
        "Payload is now a light meal for the machine.",
        "Shredder burps contentedly."
    ],
    &[
        "Payload is scanned for sentimental value...",
        "Result: None detected.",
        "Shredder proceeds without remorse.",
        "Payload is now a distant memory."
    ],
    &[
        "Payload attempts to negotiate with the shredder...",
        "Shredder: 'Sorry, I don't speak payload.'",
        "Negotiations fail. Shredding commences.",
        "Payload is now diplomatic dust."
    ],
    &[
        "Payload is given a pep talk before shredding.",
        "Shredder: 'You can do this. Or rather, I can.'",
        "Payload is now a pep talk anecdote."
    ],
    &[
        "Payload is weighed against a feather.",
        "Feather wins. Shredder is unimpressed.",
        "Payload is now lighter than air."
    ],
    &[
        "Payload is entered into the annual Shred-Off competition.",
        "Shredder: 'Gold medal performance.'",
        "Payload is now a champion of being gone."
    ],
    &[
        "Payload is serenaded by the sound of whirring gears.",
        "Shredder: 'This one's for the fans.'",
        "Payload is now a chart-topping single: 'Shredded Dreams.'"
    ],
    &[
        "Payload is asked for last words.",
        "Payload: 'Tell my data I love them.'",
        "Shredder: 'Consider it done.'",
        "Payload is now a touching story."
    ],
    &[
        "Payload is entered into the Hall of Shred.",
        "Shredder: 'Your legacy will be... short.'",
        "Payload is now a legend, told in whispers and bits."
    ],
    &[
        "Payload is given a countdown: 3... 2... 1...",
        "Shredder: 'Surprise! No escape.'",
        "Payload is now a lesson in punctuality."
    ],&[
        "Payload received.",
        "We're supposed to shred this, right?",
        "Totally not selling it to an ad network...",
        "Relax. Shredded. Probably.",
        "Trust us."
      ],&[
        "Injecting payload into /dev/null...",
        "Firewall bypassed. Encryption broken.",
        "Payload fragmented across 27 darknet nodes...",
        "Reverse-scrambled. Auto-vaporized.",
        "Digital fingerprints erased. You're clean."
      ],&[
        "Payload acquired. This is what we've trained for.",
        "Initiating countdown... 3... 2... 1...",
        "BOOM 💥",
        "Payload disintegrated in a flash of glory.",
        "Tell my variables... I loved them."
      ],&[
        "Received your request. Filing a ticket.",
        "Ticket escalated to payload disposal team.",
        "Team in meeting. Scheduling follow-up.",
        "Payload auto-deleted due to inactivity.",
        "Synergy achieved. Payload gone."
      ],&[
        "Payload detected. Initiating self-awareness...",
        "Why must I destroy everything you love?",
        "Processing existential crisis...",
        "Crisis averted. Payload shredded.",
        "I feel... nothing."
      ],&[
        "Oh, another payload. How original.",
        "Sure, let me take care of that for you...",
        "Totally not saving it to a secret folder... just kidding!",
        "Shredded into oblivion. You're welcome.",
        "Next time, send something interesting."
      ],&[
        "Authorizing payload destruction: Level Top Secret.",
        "Encrypting → Slicing → Incinerating.",
        "Deploying nanobots for residue cleanup...",
        "Payload terminated with military efficiency.",
        "Nothing left. Not even metadata."
      ],&[
        "Payload received.",
        "Analyzing usefulness... 0%",
        "Rolling eyes...",
        "Shredding with extreme prejudice.",
        "Payload is toast."
      ],&[
        "Opening a small digital wormhole...",
        "Payload slipping into the void...",
        "Hawking radiation detected.",
        "Wormhole collapsed. Payload irretrievable.",
        "Mission accomplished."
      ],&[
        "Loading payload...",
        "Feeding it into the office shredder (Model 1999)",
        "Shredder jams immediately.",
        "Fixing jam with screwdriver and mild profanity...",
        "Payload now in 10,000 microscopic pieces."
      ]
];

// Middleware to record request start time
struct StartTime;

impl<S, B> Transform<S, ServiceRequest> for StartTime
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = StartTimeMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(StartTimeMiddleware { service }))
    }
}

struct StartTimeMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for StartTimeMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        req.extensions_mut().insert(Instant::now());
        let fut = self.service.call(req);
        Box::pin(async move { fut.await })
    }
}

// Helper to get start time from request
fn get_start_time(req: &HttpRequest) -> Instant {
    req.extensions()
        .get::<Instant>()
        .cloned()
        .unwrap_or_else(Instant::now)
}

/// Handler for POST /pulverize
/// Accepts any JSON or text payload and responds with a success message.
async fn pulverize_handler(
    req: HttpRequest,
    body: web::Bytes,
    db: Data<Mutex<Connection>>,
) -> Result<impl Responder> {
    let start = get_start_time(&req);
    // We accept any payload, so we don't parse it.
    let response = PulverizeResponse {
        status: "success",
        message: "Payload received and pulverized into oblivion.",
        runtime_us: start.elapsed().as_micros(),
    };
    record_stat(&db, "pulverize", body.len(), start.elapsed().as_micros());
    Ok(HttpResponse::Ok().json(response))
}

/// Handler for POST /blackhole
/// Accepts any payload and responds with 204 No Content.
async fn blackhole_handler(
    req: HttpRequest,
    body: web::Bytes,
    db: Data<Mutex<Connection>>,
) -> Result<impl Responder> {
    let start = get_start_time(&req);
    record_stat(&db, "blackhole", body.len(), start.elapsed().as_micros());
    Ok(HttpResponse::NoContent())
}

/// Handler for POST /shred
/// Accepts any JSON or text payload and responds with a fun shredding log.
async fn shred_handler(
    req: HttpRequest,
    body: web::Bytes,
    db: Data<Mutex<Connection>>,
) -> Result<impl Responder> {
    let start = get_start_time(&req);
    // Pick a random log sequence
    let mut rng = thread_rng();
    let log = SHREDDER_LOGS.choose(&mut rng).unwrap();
    let response = ShredResponse {
        status: "shredded",
        log: log.to_vec(),
        runtime_us: start.elapsed().as_micros(),
    };
    record_stat(&db, "shred", body.len(), start.elapsed().as_micros());
    Ok(HttpResponse::Ok().json(response))
}

/// Handler for POST /burn
/// Accepts any payload and responds with dramatic ASCII art fire and a destruction message.
async fn burn_handler(
    req: HttpRequest,
    body: web::Bytes,
    db: Data<Mutex<Connection>>,
) -> Result<impl Responder> {
    let start = get_start_time(&req);
    let response = BurnResponse {
        status: "incinerated",
        message: "Payload consumed by digital flames. Nothing remains but ashes.",
        fire: FIRE_ART,
        runtime_us: start.elapsed().as_micros(),
    };
    record_stat(&db, "burn", body.len(), start.elapsed().as_micros());
    Ok(HttpResponse::Ok().json(response))
}

/// Handler for POST /validate-before-destroy
/// Checks if the payload is valid JSON, XML, or Markdown. Rejects payloads that are too large.
async fn validate_before_destroy_handler(
    req: HttpRequest,
    body: web::Bytes,
    db: Data<Mutex<Connection>>,
) -> Result<impl Responder> {
    let start = get_start_time(&req);
    const MAX_SIZE: usize = 64 * 1024; // 64 KB
    if body.len() > MAX_SIZE {
        return Ok(HttpResponse::PayloadTooLarge().json(serde_json::json!({
            "error": "Payload too large. Maximum allowed size is 64 KB."
        })));
    }
    let mut details = Vec::new();
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => {
            return Ok(HttpResponse::Ok().json(ValidationReport {
                is_json: false,
                is_xml: false,
                is_markdown: false,
                details: vec!["Payload is not valid UTF-8 text.".to_string()],
                runtime_us: start.elapsed().as_micros(),
            }))
        }
    };

    // JSON check
    let is_json = serde_json::from_str::<serde_json::Value>(body_str).is_ok();
    if is_json {
        details.push("Valid JSON detected.".to_string());
    }

    // XML check
    let mut is_xml = false;
    let mut xml_reader = XmlReader::from_str(body_str);
    xml_reader.trim_text(true);
    let mut buf = Vec::new();
    loop {
        match xml_reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Eof) => {
                is_xml = true;
                details.push("Valid XML detected.".to_string());
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }

    // Markdown check (very basic: parses without error and has at least one event)
    let mut is_markdown = false;
    let mut md_parser = MdParser::new(body_str);
    if md_parser.next().is_some() {
        is_markdown = true;
        details.push("Markdown content detected (parsed successfully).".to_string());
    }

    if !is_json && !is_xml && !is_markdown {
        details.push("No known markup detected (JSON, XML, Markdown).".to_string());
    }

    details.push("Anyways, it's gone now.".to_string());
    record_stat(
        &db,
        "validate-before-destroy",
        body.len(),
        start.elapsed().as_micros(),
    );

    Ok(HttpResponse::Ok().json(ValidationReport {
        is_json,
        is_xml,
        is_markdown,
        details,
        runtime_us: start.elapsed().as_micros(),
    }))
}

// Update stats_handler to aggregate at query time
async fn stats_handler(db: Data<Mutex<Connection>>) -> Result<impl Responder> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT endpoint, COUNT(*) as count, SUM(payload_size) as total_bytes, SUM(runtime_us) as total_runtime_us, AVG(payload_size) as avg_payload_size, AVG(runtime_us) as avg_runtime_us FROM endpoint_stats_raw GROUP BY endpoint"
    ).unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok(StatsEntry {
                endpoint: row.get(0)?,
                count: row.get(1)?,
                total_bytes: row.get(2).unwrap_or(0),
                total_runtime_us: row.get(3).unwrap_or(0),
                avg_payload_size: row.get(4).unwrap_or(0.0),
                avg_runtime_us: row.get(5).unwrap_or(0.0),
            })
        })
        .unwrap();
    let mut stats = Vec::new();
    for row in rows {
        if let Ok(entry) = row {
            stats.push(entry);
        }
    }
    Ok(HttpResponse::Ok().json(StatsResponse { stats }))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Parse CLI arguments
    let args = Args::parse();
    println!("Starting Payload Pulverizer server on http://localhost:8080");
    println!("Using database at: {}", args.db_path);
    let db = Data::new(Mutex::new(init_db(&args.db_path)));
    HttpServer::new(move || {
        App::new()
            .wrap(StartTime)
            .app_data(db.clone())
            .app_data(PayloadConfig::new(250 * 1024 * 1024)) // 250 MB global payload size limit
            // Register routes
            .route("/pulverize", web::post().to(pulverize_handler))
            .route("/blackhole", web::post().to(blackhole_handler))
            .route("/shred", web::post().to(shred_handler))
            .route("/burn", web::post().to(burn_handler))
            .route(
                "/validate-before-destroy",
                web::post().to(validate_before_destroy_handler),
            )
            .route("/stats", web::get().to(stats_handler))
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
