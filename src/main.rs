#[macro_use]
extern crate rocket;

use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use rand::prelude::*;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
    serde::{json::Json, Deserialize, Serialize},
    tokio::{
        self, fs,
        time::{self, Duration},
    },
};

const CONFIG_FILE: &str = "config.toml";

struct ApiKey(String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ApiKey {
    type Error = String;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        async fn is_valid(key: &str) -> Result<bool, (Status, String)> {
            let config = read_config_file().await?;
            Ok(key == config.api_key)
        }

        match request.headers().get_one("x-api-key") {
            None => Outcome::Failure((Status::BadRequest, "missing x-api-key".to_string())),
            Some(key)
                if match is_valid(key).await {
                    Ok(b) => b,
                    Err(e) => return Outcome::Failure(e),
                } =>
            {
                Outcome::Success(ApiKey(key.to_string()))
            }
            Some(_) => {
                Outcome::Failure((Status::UnprocessableEntity, "invalid x-api-key".to_string()))
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum LightState {
    On,
    Off,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
struct FlipSettings {
    delay: u64, // ms
    servo_value: f32,
}

impl FlipSettings {
    fn updated(&self, other: &FlipSettingsUpdate) -> FlipSettings {
        FlipSettings {
            delay: match other.delay {
                Some(delay) => delay,
                None => self.delay,
            },
            servo_value: match other.servo_value {
                Some(servo_value) => servo_value,
                None => self.servo_value,
            },
        }
    }

    fn update(&mut self, other: &FlipSettingsUpdate) {
        *self = self.updated(other);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FlipSettingsUpdate {
    delay: Option<u64>,
    servo_value: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ScheduledFlip {
    state: LightState,
    #[serde(with = "ts_seconds")]
    datetime: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    idle_servo_value: f32,
    api_key: String,
    scheduled_flip: Option<ScheduledFlip>,
    on_settings: FlipSettings,
    off_settings: FlipSettings,
}

fn generate_api_key() -> String {
    let mut data = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut data);
    base64::encode(data)
}

async fn write_config_file(config: &Config) -> Result<(), (Status, String)> {
    fs::write(
        CONFIG_FILE,
        toml::to_string(&config).map_err(|e| (Status::InternalServerError, e.to_string()))?,
    )
    .await
    .map_err(|e| (Status::InternalServerError, e.to_string()))
}

async fn read_config_file() -> Result<Config, (Status, String)> {
    match fs::read_to_string(CONFIG_FILE).await {
        Ok(s) => toml::from_str(&s).map_err(|e| (Status::InternalServerError, e.to_string())),
        Err(_) => {
            let config = Config {
                idle_servo_value: 0.0,
                scheduled_flip: None,
                on_settings: FlipSettings {
                    delay: 0,
                    servo_value: 0.0,
                },
                off_settings: FlipSettings {
                    delay: 0,
                    servo_value: 0.0,
                },
                api_key: generate_api_key(),
            };
            write_config_file(&config).await.and(Ok(config))
        }
    }
}

fn set(servo_value: f32) {
    println!("Turned switch to value {servo_value}");
}

async fn flip(settings: &FlipSettings, config: &Config) {
    set(settings.servo_value);
    time::sleep(Duration::from_millis(settings.delay)).await;
    set(config.idle_servo_value);
}

async fn run_schedules() {
    println!("run");
    if let Ok(mut config) = read_config_file().await {
        if let Some(scheduled_flip) = config.scheduled_flip {
            if scheduled_flip.datetime <= Utc::now() {
                config.scheduled_flip = None;
                write_config_file(&config).await.ok();
                flip(
                    &match scheduled_flip.state {
                        LightState::On => config.on_settings,
                        LightState::Off => config.off_settings,
                    },
                    &config,
                )
                .await;
            }
        }
    }
}

#[get("/")]
fn index() -> &'static str {
    "WIP"
}

#[patch("/light-state", data = "<state>")]
async fn patch_light_state(state: Json<LightState>, _key: ApiKey) -> Result<(), (Status, String)> {
    let config = read_config_file().await?;
    flip(
        &match state.into_inner() {
            LightState::On => config.on_settings,
            LightState::Off => config.off_settings,
        },
        &config,
    )
    .await;
    Ok(())
}

#[post("/schedule", data = "<scheduled_flip>")]
async fn post_schedule(
    scheduled_flip: Json<ScheduledFlip>,
    _key: ApiKey,
) -> Result<(), (Status, String)> {
    if scheduled_flip.datetime <= Utc::now() {
        return Err((
            Status::UnprocessableEntity,
            "datetime must be in the future".to_string(),
        ));
    }
    let mut config = read_config_file().await?;
    config.scheduled_flip = Some(scheduled_flip.into_inner());
    write_config_file(&config).await
}

#[patch("/settings/test", data = "<settings>")]
async fn patch_settings_test(
    settings: Json<FlipSettings>,
    _key: ApiKey,
) -> Result<(), (Status, String)> {
    flip(&settings.into_inner(), &read_config_file().await?).await;
    Ok(())
}

#[get("/settings/on")]
async fn get_settings_on() -> Result<Json<FlipSettings>, (Status, String)> {
    Ok(Json(read_config_file().await?.on_settings))
}

#[patch("/settings/on", data = "<settings>")]
async fn patch_settings_on(
    settings: Json<FlipSettingsUpdate>,
    _key: ApiKey,
) -> Result<(), (Status, String)> {
    let mut config = read_config_file().await?;
    config.on_settings.update(&settings.into_inner());
    write_config_file(&config).await
}

#[get("/settings/off")]
async fn get_settings_off() -> Result<Json<FlipSettings>, (Status, String)> {
    Ok(Json(read_config_file().await?.off_settings))
}

#[patch("/settings/off", data = "<settings>")]
async fn patch_settings_off(
    settings: Json<FlipSettingsUpdate>,
    _key: ApiKey,
) -> Result<(), (Status, String)> {
    let mut config = read_config_file().await?;
    config.off_settings.update(&settings.into_inner());
    write_config_file(&config).await
}

#[get("/settings/idle")]
async fn get_settings_idle() -> Result<Json<f32>, (Status, String)> {
    Ok(Json(read_config_file().await?.idle_servo_value))
}

#[patch("/settings/idle", data = "<value>")]
async fn patch_settings_idle(value: Json<f32>, _key: ApiKey) -> Result<(), (Status, String)> {
    let value = value.into_inner();
    let mut config = read_config_file().await?;
    config.idle_servo_value = value;
    set(value);
    write_config_file(&config).await
}

#[launch]
async fn rocket() -> _ {
    let config = read_config_file().await.unwrap(); // panic if the config file is badly formatted before we start rocket
    set(config.idle_servo_value);

    tokio::spawn(async {
        loop {
            time::sleep(Duration::from_secs(60)).await;
            run_schedules().await;
        }
    });

    rocket::build().mount("/", routes![index]).mount(
        "/api/v0/",
        routes![
            patch_light_state,
            post_schedule,
            patch_settings_test,
            get_settings_on,
            patch_settings_on,
            get_settings_off,
            patch_settings_off,
            get_settings_idle,
            patch_settings_idle
        ],
    )
}
