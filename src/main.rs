#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

use std::env;
use std::fmt;
use std::panic;
use std::process;
use std::thread;

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use futures_util::stream::SplitSink;

use rocket::State;
use rocket::fairing::AdHoc;
use rocket::http::{RawStr, Status};
use rocket::request::{Form, FromFormValue};
use rocket::response::{NamedFile, Redirect};

use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

use rosc::{OscPacket, OscType};

use rppal::gpio::{Gpio, OutputPin};

use serde::{Deserialize, Serialize};

use serde_with::{serde_as, DurationMilliSeconds};

use tokio::net::{TcpListener, TcpStream};
use tokio::runtime;

use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Error as WSError, Message as WSMessage};

use yansi::Paint;


#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

enum ColorError {
    BadFormat,
    ParseError,
}

impl FromStr for Color {
    type Err = ColorError;

    fn from_str(color: &str) -> Result<Self, Self::Err> {
        if &color[0..1] != "#" || color.len() != 7 {
            return Err(Self::Err::BadFormat);
        }

        let result = || -> Result<Color, ParseIntError> {
            let red = u8::from_str_radix(&color[1..3], 16)?;
            let green = u8::from_str_radix(&color[3..5], 16)?;
            let blue = u8::from_str_radix(&color[5..7], 16)?;

            Ok(Color { red, green, blue })
        }();

        match result {
            Ok(color) => {
                return Ok(color);
            },
            Err(_err) => {
                return Err(Self::Err::ParseError);
            }
        }
    }
}

impl<'v> FromFormValue<'v> for Color {
    type Error = ColorError;

    fn from_form_value(value: &'v RawStr) -> Result<Self, Self::Error> {
        match value.url_decode() {
            Ok(color) => {
                return Color::from_str(&color);
            },
            Err(_err) => {
                return Err(Self::Error::BadFormat);
            }
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
}

#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
struct Frame {
    color: Color,
    #[serde_as(as = "DurationMilliSeconds")]
    duration: Duration,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "content")]
enum Pattern {
    Off,
    Solid(Color),
    Custom(Vec<Frame>),
}

struct Output {
    frequency: f64,

    red: OutputPin,
    green: OutputPin,
    blue: OutputPin,
}

impl Output {
    fn set(&mut self, color: Color) -> rppal::gpio::Result<()> {
        self.red.set_pwm_frequency(self.frequency, color.red as f64 / 255.0)?;
        self.green.set_pwm_frequency(self.frequency, color.green as f64 / 255.0)?;
        self.blue.set_pwm_frequency(self.frequency, color.blue as f64 / 255.0)?;

        Ok(())
    }
}

struct Lights {
    output: Output,
    pattern: Pattern,

    frame: usize,
    instant: Instant,

    last: Color,
}

impl Lights {
    fn new(output: Output, pattern: Pattern) -> Lights {
        let mut lights = Lights {
            output,
            pattern,

            frame: 0,
            instant: Instant::now(),
            last: Color { red: 0, green: 0, blue: 0 },
        };

        lights.output.set(Color { red: 0, green: 0, blue: 0 }).expect("Lights output failure");

        lights
    }

    fn get(&self) -> Color {
        match &self.pattern {
            Pattern::Off => {
                Color { red: 0, green: 0, blue: 0 }
            },
            Pattern::Solid(color) => {
                *color
            },
            Pattern::Custom(frames) => {
                if frames.len() > 0 {
                    frames[self.frame].color
                }
                else {
                    Color { red: 0, green: 0, blue: 0 }
                }
            },
        }
    }

    fn set(&mut self, color: Color) {
        self.pattern = Pattern::Solid(color);
    }

    fn get_pattern(&self) -> &Pattern {
        &self.pattern
    }

    fn set_pattern(&mut self, pattern: &Pattern) {
        self.pattern = pattern.clone();
    }

    fn tick(&mut self) {
        let next = match &self.pattern {
            Pattern::Off => {
                Color { red: 0, green: 0, blue: 0 }
            },
            Pattern::Solid(color) => {
                *color
            },
            Pattern::Custom(frames) => {
                if frames.len() > 0 {
                    if self.frame >= frames.len() {
                        self.frame = 0;
                    }

                    while self.instant.elapsed() >= frames[self.frame].duration {
                        self.instant = self.instant.checked_add(frames[self.frame].duration).unwrap();
                        self.frame = (self.frame + 1) % frames.len();
                    }

                    frames[self.frame].color
                }
                else {
                    self.instant = Instant::now();
                    self.frame = 0;

                    Color { red: 0, green: 0, blue: 0 }
                }
            },
        };

        if next != self.last {
            self.output.set(next).expect("Lights output failure");
            self.last = next;
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct APIError {
    status: String,
    message: String,
}

#[derive(FromForm)]
struct ColorForm {
    color: Color,
}

type SharedLights = Arc<Mutex<Lights>>;

#[get("/color")]
fn get_color(lights: State<SharedLights>) -> Json<Color> {
    Json(lights.lock().unwrap().get())
}

#[put("/color", data = "<color>")]
fn set_color(color: Json<Color>, lights: State<SharedLights>) -> Status {
    lights.lock().unwrap().set(*color);

    Status::NoContent
}

#[get("/pattern")]
fn get_pattern(lights: State<SharedLights>) -> Json<Pattern> {
    Json(lights.lock().unwrap().get_pattern().clone())
}

#[put("/pattern", data = "<pattern>")]
fn set_pattern(pattern: Json<Pattern>, lights: State<SharedLights>) -> Status {
    lights.lock().unwrap().set_pattern(&pattern);

    Status::NoContent
}

#[get("/wsinfo")]
fn ws_info() -> String {
    match env::var("WS_INFO") {
        Ok(val) => val,
        Err(_err) => String::from("")
    }
}

#[get("/static/<file..>")]
fn files(file: PathBuf) -> Option<NamedFile> {
    NamedFile::open(Path::new("static/").join(file)).ok()
}

#[get("/")]
fn form(lights: State<SharedLights>) -> Template {
    let context = [
        (String::from("color"), lights.lock().unwrap().get().to_string()),
    ];

    Template::render("form", context.iter().cloned().collect::<HashMap<String, String>>())
}

#[post("/", data = "<color_form>")]
fn form_submit(color_form: Form<ColorForm>, lights: State<SharedLights>) -> Redirect {
    lights.lock().unwrap().set(color_form.color);

    Redirect::to(uri!(form))
}

#[catch(400)]
fn bad_request() -> Json<APIError> {
    Json(APIError {
        status: String::from("error"),
        message: String::from("Malformed request"),
    })
}

#[catch(422)]
fn unprocessable_entity() -> Json<APIError> {
    Json(APIError {
        status: String::from("error"),
        message: String::from("Malformed request"),
    })
}

#[catch(404)]
fn not_found() -> Json<APIError> {
    Json(APIError {
        status: String::from("error"),
        message: String::from("Resource not found"),
    })
}

fn ws_server(lights: SharedLights, chronon: Duration) {
    let address = match env::var("WS_ADDRESS") {
        Ok(val) => val,
        Err(_err) => String::from(if cfg!(debug_assertions) { "127.0.0.1" } else { "0.0.0.0" })
    };

    let port: u16 = match env::var("WS_PORT") {
        Ok(val) => val.parse().unwrap(),
        Err(_err) => 8001
    };

    let runtime = runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap();

    runtime.block_on(async move {
        let listener = TcpListener::bind((address, port)).await.expect("Failed to bind TCP WebSocket address");

        println!("{}{} {}", Paint::masked("🕸  "), Paint::default("WebSocket server started on").bold(), Paint::default(String::from("ws://") + &listener.local_addr().unwrap().to_string()).bold().underline());

        let streams = Arc::new(Mutex::new(HashMap::<SocketAddr, SplitSink<WebSocketStream<TcpStream>, WSMessage>>::new()));

        let mut last_color = lights.lock().unwrap().get();

        let mut interval = tokio::time::interval(chronon);

        loop {
            tokio::select! {
                connection = listener.accept() => {
                    match connection {
                        Ok((socket, _)) => {
                            match tokio_tungstenite::accept_async(socket).await {
                                Ok(stream) => {
                                    let peer = stream.get_ref().peer_addr().unwrap();

                                    let (sender, mut receiver) = stream.split();

                                    streams.lock().unwrap().insert(peer, sender);

                                    let lights_conn = Arc::clone(&lights);
                                    let streams_conn = Arc::clone(&streams);

                                    tokio::spawn(async move {
                                        loop {
                                            match receiver.next().await {
                                                Some(Ok(WSMessage::Text(string))) => {
                                                    match serde_json::from_str::<Color>(&string) {
                                                        Ok(color) => {
                                                            lights_conn.lock().unwrap().set(color);
                                                        },
                                                        Err(err) => {
                                                            eprintln!("Failed to parse color from WebSocket: {}", err);
                                                        }
                                                    }
                                                },
                                                Some(Ok(WSMessage::Close(_frame))) => {
                                                    break;
                                                },
                                                Some(Ok(_)) => {
                                                    // ignore other message types
                                                },
                                                Some(Err(WSError::Protocol(ref err))) if err == "Connection reset without closing handshake" => {
                                                    // resets seem to be common for browsers
                                                    break;
                                                },
                                                Some(Err(err)) => {
                                                    eprintln!("Failed to poll WebSocket connection: {}", err);
                                                    break;
                                                },
                                                None => {
                                                    break;
                                                }
                                            }
                                        }

                                        let removed = streams_conn.lock().unwrap().remove(&peer);

                                        match removed {
                                            Some(mut stream) => {
                                                match stream.close().await {
                                                    Ok(()) => {},
                                                    Err(err) => {
                                                        eprintln!("Failed to close WebSocket connection: {}", err);
                                                    }
                                                }
                                            },
                                            None => {}
                                        }
                                    });
                                },
                                Err(err) => {
                                    eprintln!("Failed to accept WebSocket connection: {}", err);
                                }
                            }
                        },
                        Err(err) => {
                            eprintln!("Failed to accept WebSocket connection: {}", err);
                        }
                    }
                }

                _ = interval.tick() => {
                    let color = lights.lock().unwrap().get();

                    if color != last_color {
                        let string = serde_json::to_string(&color).unwrap();

                        for (_, stream) in streams.lock().unwrap().iter_mut() {
                            match stream.send(WSMessage::Text(string.clone())).await {
                                Ok(_) => {},
                                Err(err) => {
                                    // task should handle removal on I/O errors
                                    eprintln!("Failed to send color to WebSocket: {}", err);
                                }
                            }
                        }

                        last_color = color;
                    }
                }
            }
        }
    });
}

fn osc_server(lights: SharedLights) {
    let address = match env::var("OSC_ADDRESS") {
        Ok(val) => val,
        Err(_err) => String::from(if cfg!(debug_assertions) { "127.0.0.1" } else { "0.0.0.0" })
    };

    let port: u16 = match env::var("OSC_PORT") {
        Ok(val) => val.parse().unwrap(),
        Err(_err) => 1337
    };

    let socket = UdpSocket::bind((address, port)).expect("Failed to bind UDP OSC address");

    println!("{}{} {}", Paint::masked("🎛  "), Paint::default("OSC server started on").bold(), Paint::default(socket.local_addr().unwrap()).bold().underline());

    let mut buffer = [0u8; rosc::decoder::MTU];

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((size, _addr)) => {
                match rosc::decoder::decode(&buffer[..size]) {
                    Ok(packet) => {
                        match packet {
                            OscPacket::Message(msg) => {
                                match msg.addr.as_ref() {
                                    "/color" => {
                                        match &msg.args[..] {
                                            [OscType::Int(red), OscType::Int(green), OscType::Int(blue)] => {
                                                lights.lock().unwrap().set(Color { red: *red as u8, green: *green as u8, blue: *blue as u8 });
                                            },
                                            [OscType::Float(red), OscType::Float(green), OscType::Float(blue)] => {
                                                lights.lock().unwrap().set(Color { red: *red as u8, green: *green as u8, blue: *blue as u8 });
                                            },
                                            [OscType::Double(red), OscType::Double(green), OscType::Double(blue)] => {
                                                lights.lock().unwrap().set(Color { red: *red as u8, green: *green as u8, blue: *blue as u8 });
                                            },
                                            [OscType::Color(color)] => {
                                                lights.lock().unwrap().set(Color { red: color.red, green: color.green, blue: color.blue });
                                            },
                                            _ => {
                                                eprintln!("Unexpected OSC /color command: {:?}", msg.args);
                                            }
                                        }
                                    },
                                    "/pattern/off" => {
                                        match &msg.args[..] {
                                            [] => {
                                                lights.lock().unwrap().set_pattern(&Pattern::Off);
                                            },
                                            _ => {
                                                eprintln!("Unexpected OSC /pattern/off command: {:?}", msg.args);
                                            }
                                        }
                                    },
                                    "/pattern/solid" => {
                                        match &msg.args[..] {
                                            [OscType::Int(red), OscType::Int(green), OscType::Int(blue)] => {
                                                lights.lock().unwrap().set_pattern(&Pattern::Solid(Color { red: *red as u8, green: *green as u8, blue: *blue as u8 }));
                                            },
                                            [OscType::Float(red), OscType::Float(green), OscType::Float(blue)] => {
                                                lights.lock().unwrap().set_pattern(&Pattern::Solid(Color { red: *red as u8, green: *green as u8, blue: *blue as u8 }));
                                            },
                                            [OscType::Double(red), OscType::Double(green), OscType::Double(blue)] => {
                                                lights.lock().unwrap().set_pattern(&Pattern::Solid(Color { red: *red as u8, green: *green as u8, blue: *blue as u8 }));
                                            },
                                            [OscType::Color(color)] => {
                                                lights.lock().unwrap().set_pattern(&Pattern::Solid(Color { red: color.red, green: color.green, blue: color.blue }));
                                            },
                                            _ => {
                                                eprintln!("Unexpected OSC /pattern/solid command: {:?}", msg.args);
                                            }
                                        }
                                    },
                                    _ => {
                                        eprintln!("Unexpected OSC Message: {}: {:?}", msg.addr, msg.args);
                                    }
                                }
                            },
                            OscPacket::Bundle(bundle) => {
                                eprintln!("Unexpected OSC Bundle: {:?}", bundle);
                            },
                        }
                    },
                    Err(err) => {
                        eprintln!("Error decoding OSC packet: {:?}", err);
                    }
                }
            },
            Err(err) => {
                eprintln!("Error receiving from socket: {}", err);
            }
        }
    }
}

fn pattern_output(lights: SharedLights, chronon: Duration) {
    println!("{}{}", Paint::masked("💡 "), Paint::default("Light pattern output started").bold());

    loop {
        lights.lock().unwrap().tick();
        thread::sleep(chronon);
    }
}

fn main() {
    let initial = Color { red: 242, green: 155, blue: 212 };

    let chronon = Duration::from_millis(10);

    let gpio = Gpio::new().unwrap();

    let lights = Arc::new(Mutex::new(Lights::new(
        Output {
            frequency: 60.0,

            red: gpio.get(17).unwrap().into_output(),
            green: gpio.get(27).unwrap().into_output(),
            blue: gpio.get(22).unwrap().into_output(),
        },
        Pattern::Solid(initial),
    )));

    let lights_rocket = Arc::clone(&lights);
    let lights_ws = Arc::clone(&lights);
    let lights_osc = Arc::clone(&lights);
    let lights_output = Arc::clone(&lights);

    let chronon_ws = chronon.clone();
    let chronon_output = chronon.clone();

    let orig_panic_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        orig_panic_hook(info);
        process::exit(1);
    }));

    rocket::ignite()
        .mount("/", routes![get_color, set_color, get_pattern, set_pattern, ws_info, files, form, form_submit])
        .register(catchers![bad_request, unprocessable_entity, not_found])
        .manage(lights_rocket)
        .attach(Template::fairing())
        .attach(AdHoc::on_launch("WebSocket Server", move |_rocket| {
            thread::spawn(move || {
                ws_server(lights_ws, chronon_ws);
            });
        }))
        .attach(AdHoc::on_launch("OSC Server", move |_rocket| {
            thread::spawn(move || {
                osc_server(lights_osc);
            });
        }))
        .attach(AdHoc::on_launch("Light Pattern Output", move |_rocket| {
            thread::spawn(move || {
                pattern_output(lights_output, chronon_output);
            });
        }))
        .launch();
}
