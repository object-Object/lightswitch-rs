#[macro_use]
extern crate rocket;

use chrono::Utc;
use rocket::tokio::{
    self,
    time::{self, Duration},
};
use rppal::pwm::Pwm;
use std::sync::Arc;

pub mod config {
    use chrono::{serde::ts_seconds, DateTime, LocalResult, TimeZone, Utc};
    use rand::prelude::*;
    use rocket::{
        form::{self, FromForm, FromFormField},
        http::Status,
        serde::{Deserialize, Serialize},
        tokio::fs,
    };

    const CONFIG_FILE: &str = "config.toml";

    #[derive(Debug, Serialize, Deserialize)]
    pub struct FormDateTime {
        #[serde(with = "ts_seconds")]
        pub inner: DateTime<Utc>,
    }

    impl<'r> FromFormField<'r> for FormDateTime {
        fn from_value(field: form::ValueField<'r>) -> form::Result<'r, Self> {
            Ok(FormDateTime {
                inner: match Utc.timestamp_opt(field.value.parse::<i64>()?, 0) {
                    LocalResult::Single(datetime) => datetime,
                    LocalResult::None => {
                        return Err(form::Error::validation("invalid timestamp").into())
                    }
                    LocalResult::Ambiguous(_, _) => unreachable!(),
                },
            })
        }
    }

    #[derive(Debug, Serialize, Deserialize, FromFormField)]
    pub enum LightState {
        On,
        Off,
    }

    impl LightState {
        pub fn get_settings<'a>(&self, config: &'a Config) -> &'a FlipSettings {
            match *self {
                Self::On => &config.on_settings,
                Self::Off => &config.off_settings,
            }
        }
    }

    #[derive(Debug, Serialize, Deserialize, FromForm)]
    pub struct FlipSettings {
        pub delay: u64, // ms
        #[field(validate = servo_value_validate())]
        pub servo_value: f64,
    }

    #[derive(Debug, Serialize, Deserialize, FromForm)]
    pub struct ScheduledFlip {
        pub state: LightState,
        #[field(validate = formdatetime_validate())]
        pub datetime: FormDateTime,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Config {
        pub idle_servo_value: f64,
        pub api_key: String,
        pub scheduled_flip: Option<ScheduledFlip>,
        pub on_settings: FlipSettings,
        pub off_settings: FlipSettings,
    }

    fn servo_value_validate<'v>(servo_value: &f64) -> form::Result<'v, ()> {
        if *servo_value < -1.0 || *servo_value > 1.0 {
            return Err(
                form::Error::validation("invalid servo value, must be between -1 and 1").into(),
            );
        }
        Ok(())
    }

    fn formdatetime_validate<'v>(formdatetime: &FormDateTime) -> form::Result<'v, ()> {
        if formdatetime.inner <= Utc::now() {
            return Err(form::Error::validation("invalid datetime, must be in the future").into());
        }
        Ok(())
    }

    fn generate_api_key() -> String {
        let mut data = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut data);
        base64::encode(data)
    }

    pub async fn write_config_file(config: &Config) -> Result<(), (Status, String)> {
        fs::write(
            CONFIG_FILE,
            toml::to_string(&config).map_err(|e| (Status::InternalServerError, e.to_string()))?,
        )
        .await
        .map_err(|e| (Status::InternalServerError, e.to_string()))
    }

    pub async fn read_config_file() -> Result<Config, (Status, String)> {
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
}

pub mod servo {
    use crate::config::{Config, FlipSettings};
    use rocket::{
        http::Status,
        tokio::time::{self, Duration},
    };
    use rppal::pwm::{self, Channel, Polarity, Pwm};

    const PERIOD: Duration = Duration::from_millis(20);
    const PULSE_MIN_US: u64 = 1000;
    const PULSE_MAX_US: u64 = 2000;

    pub fn create_pwm(config: &Config) -> pwm::Result<Pwm> {
        Pwm::with_period(
            Channel::Pwm0,
            PERIOD,
            calc_pulse_width(config.idle_servo_value),
            Polarity::Normal,
            true,
        )
    }

    pub fn calc_pulse_width(servo_value: f64) -> Duration {
        Duration::from_micros(
            ((servo_value + 1.0) * ((PULSE_MAX_US - PULSE_MIN_US) / 2) as f64 + PULSE_MIN_US as f64)
                .round() as u64,
        )
    }

    fn to_500(e: impl ToString) -> (Status, String) {
        (Status::InternalServerError, e.to_string())
    }

    fn enable(pwm: &Pwm) -> Result<(), (Status, String)> {
        pwm.enable().map_err(to_500)
    }

    fn disable(pwm: &Pwm) -> Result<(), (Status, String)> {
        pwm.disable().map_err(to_500)
    }

    fn set(servo_value: f64, pwm: &Pwm) -> Result<(), (Status, String)> {
        pwm.set_pulse_width(calc_pulse_width(servo_value))
            .map_err(to_500)
    }

    pub async fn set_value(servo_value: f64, pwm: &Pwm) -> Result<(), (Status, String)> {
        enable(pwm)?;

        set(servo_value, pwm)?;
        time::sleep(Duration::from_millis(500)).await;

        disable(pwm)
    }

    pub async fn flip(
        settings: &FlipSettings,
        config: &Config,
        pwm: &Pwm,
    ) -> Result<(), (Status, String)> {
        enable(pwm)?;

        set(settings.servo_value, pwm)?;
        time::sleep(Duration::from_millis(settings.delay)).await;

        set(config.idle_servo_value, pwm)?;
        time::sleep(Duration::from_millis(500)).await;

        disable(pwm)
    }
}

pub mod api {
    use crate::{
        config::{self, FlipSettings, LightState, ScheduledFlip},
        servo,
    };
    use rocket::{
        form::Form,
        http::Status,
        request::{FromRequest, Outcome, Request},
        serde::json::Json,
        State,
    };
    use rppal::pwm::Pwm;
    use std::sync::Arc;

    pub struct ApiKey(String);

    #[rocket::async_trait]
    impl<'r> FromRequest<'r> for ApiKey {
        type Error = String;

        async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
            async fn is_valid(key: &str) -> Result<bool, (Status, String)> {
                let config = config::read_config_file().await?;
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
                    Outcome::Failure((Status::UnprocessableEntity, "x-api-key invalid".to_string()))
                }
            }
        }
    }

    pub mod get {
        use super::*;

        #[get("/")]
        pub fn index() -> &'static str {
            "WIP"
        }

        #[get("/settings/on")]
        pub async fn settings_on() -> Result<Json<FlipSettings>, (Status, String)> {
            Ok(Json(config::read_config_file().await?.on_settings))
        }

        #[get("/settings/off")]
        pub async fn settings_off() -> Result<Json<FlipSettings>, (Status, String)> {
            Ok(Json(config::read_config_file().await?.off_settings))
        }

        #[get("/settings/idle")]
        pub async fn settings_idle() -> Result<Json<f64>, (Status, String)> {
            Ok(Json(config::read_config_file().await?.idle_servo_value))
        }
    }

    pub mod patch {
        use super::*;

        #[patch("/light-state", data = "<state>")]
        pub async fn light_state(
            state: Form<LightState>,
            _key: ApiKey,
            pwm: &State<Arc<Pwm>>,
        ) -> Result<(), (Status, String)> {
            let config = config::read_config_file().await?;
            servo::flip(state.get_settings(&config), &config, pwm).await
        }

        #[patch("/settings/test", data = "<settings>")]
        pub async fn settings_test(
            settings: Form<FlipSettings>,
            _key: ApiKey,
            pwm: &State<Arc<Pwm>>,
        ) -> Result<(), (Status, String)> {
            servo::flip(&settings, &config::read_config_file().await?, pwm).await
        }

        #[patch("/settings/on", data = "<settings>")]
        pub async fn settings_on(
            settings: Form<FlipSettings>,
            _key: ApiKey,
        ) -> Result<(), (Status, String)> {
            let mut config = config::read_config_file().await?;
            config.on_settings = settings.into_inner();
            config::write_config_file(&config).await
        }

        #[patch("/settings/off", data = "<settings>")]
        pub async fn settings_off(
            settings: Form<FlipSettings>,
            _key: ApiKey,
        ) -> Result<(), (Status, String)> {
            let mut config = config::read_config_file().await?;
            config.off_settings = settings.into_inner();
            config::write_config_file(&config).await
        }

        #[patch("/settings/idle", data = "<value>")]
        pub async fn settings_idle(
            value: Form<f64>,
            _key: ApiKey,
            pwm: &State<Arc<Pwm>>,
        ) -> Result<(), (Status, String)> {
            let mut config = config::read_config_file().await?;
            config.idle_servo_value = *value;
            servo::set_value(*value, pwm).await?;
            config::write_config_file(&config).await
        }

        #[patch("/schedule", data = "<scheduled_flip>")]
        pub async fn schedule(
            scheduled_flip: Form<ScheduledFlip>,
            _key: ApiKey,
        ) -> Result<(), (Status, String)> {
            let mut config = config::read_config_file().await?;
            config.scheduled_flip = Some(scheduled_flip.into_inner());
            config::write_config_file(&config).await
        }
    }

    pub mod delete {
        use super::*;

        #[delete("/schedule")]
        pub async fn schedule(_key: ApiKey) -> Result<(), (Status, String)> {
            let mut config = config::read_config_file().await?;
            config.scheduled_flip = None;
            config::write_config_file(&config).await
        }
    }
}

async fn run_schedules(pwm: &Pwm) {
    if let Ok(mut config) = config::read_config_file().await {
        if let Some(scheduled_flip) = config.scheduled_flip {
            if scheduled_flip.datetime.inner <= Utc::now() {
                config.scheduled_flip = None;
                config::write_config_file(&config).await.ok();
                servo::flip(scheduled_flip.state.get_settings(&config), &config, pwm)
                    .await
                    .ok();
            }
        }
    }
}

#[launch]
async fn rocket() -> _ {
    let config = config::read_config_file().await.unwrap(); // panic if the config file is badly formatted before we start rocket

    let pwm = Arc::new(servo::create_pwm(&config).unwrap());

    let pwm2 = pwm.clone();
    tokio::spawn(async move {
        loop {
            time::sleep(Duration::from_secs(60)).await;
            run_schedules(&pwm2).await;
        }
    });

    rocket::build()
        .mount("/", routes![api::get::index])
        .mount(
            "/api/v0/",
            routes![
                api::get::settings_on,
                api::get::settings_off,
                api::get::settings_idle,
                api::patch::light_state,
                api::patch::settings_test,
                api::patch::settings_on,
                api::patch::settings_off,
                api::patch::settings_idle,
                api::patch::schedule,
                api::delete::schedule,
            ],
        )
        .manage(pwm)
}
