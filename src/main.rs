#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

use std::env;
use std::fmt;
use std::thread;

use std::collections::HashMap;
use std::net::UdpSocket;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use rocket::State;
use rocket::fairing::AdHoc;
use rocket::http::{RawStr, Status};
use rocket::request::{Form, FromFormValue};
use rocket::response::NamedFile;

use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

use rosc::{OscPacket, OscType};

use rppal::gpio::{Gpio, OutputPin};

use serde::{Deserialize, Serialize};

use yansi::Paint;


#[derive(Clone, Copy, Serialize, Deserialize)]
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
            Err(_err) => {
                return Err(Self::Err::ParseError);
            },
            Ok(color) => {
                return Ok(color);
            }
        }
    }
}

impl<'v> FromFormValue<'v> for Color {
    type Error = ColorError;

    fn from_form_value(value: &'v RawStr) -> Result<Self, Self::Error> {
        match value.url_decode() {
            Err(_err) => {
                return Err(Self::Error::BadFormat);
            },
            Ok(color) => {
                return Color::from_str(&color);
            }
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Error {
    status: String,
    message: String,
}

type CurrentColor = Arc<Mutex<Color>>;

struct Output {
    frequency: f64,
    red_pin: OutputPin,
    green_pin: OutputPin,
    blue_pin: OutputPin,
}

type CurrentOutput = Arc<Mutex<Output>>;

#[derive(FromForm)]
struct ColorForm {
    color: Color,
}

#[get("/color")]
fn get_color(current: State<CurrentColor>) -> Json<Color> {
    Json(*current.lock().unwrap())
}

#[put("/color", data = "<color>")]
fn set_color(color: Json<Color>, current: State<CurrentColor>, output: State<CurrentOutput>) -> Status {
    let mut current_color = current.lock().unwrap();
    let mut current_output = output.lock().unwrap();

    current_color.red = color.red;
    current_color.green = color.green;
    current_color.blue = color.blue;

    set_output(&mut current_output, *current_color).unwrap();

    Status::NoContent
}

#[get("/static/<file..>")]
fn files(file: PathBuf) -> Option<NamedFile> {
    NamedFile::open(Path::new("static/").join(file)).ok()
}

#[get("/")]
fn form(current: State<CurrentColor>) -> Template {
    let current_color = current.lock().unwrap();

    let context = [
        (String::from("color"), current_color.to_string()),
    ];

    Template::render("form", context.iter().cloned().collect::<HashMap<String, String>>())
}

#[post("/", data = "<color_form>")]
fn form_submit(color_form: Form<ColorForm>, current: State<CurrentColor>, output: State<CurrentOutput>) -> Template {
    {
        let mut current_color = current.lock().unwrap();
        let mut current_output = output.lock().unwrap();

        current_color.red = color_form.color.red;
        current_color.green = color_form.color.green;
        current_color.blue = color_form.color.blue;

        set_output(&mut current_output, *current_color).unwrap();
    }

    form(current)
}

#[catch(400)]
fn bad_request() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Malformed request"),
    })
}

#[catch(422)]
fn unprocessable_entity() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Malformed request"),
    })
}

#[catch(404)]
fn not_found() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Resource not found"),
    })
}

fn osc_server(color: CurrentColor, output: CurrentOutput) {
    let address = match env::var("OSC_ADDRESS") {
        Ok(val) => val,
        Err(_err) => String::from(if cfg!(debug_assertions) { "127.0.0.1" } else { "0.0.0.0" }),
    };

    let port: u16 = match env::var("OSC_PORT") {
        Ok(val) => val.parse().unwrap(),
        Err(_err) => 1337,
    };

    let socket = UdpSocket::bind((address, port)).unwrap();

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
                                        let mut current_color = color.lock().unwrap();
                                        let mut current_output = output.lock().unwrap();

                                        match &msg.args[..] {
                                            [OscType::Int(red), OscType::Int(green), OscType::Int(blue)] => {
                                                current_color.red = *red as u8;
                                                current_color.green = *green as u8;
                                                current_color.blue = *blue as u8;
                                            },
                                            [OscType::Float(red), OscType::Float(green), OscType::Float(blue)] => {
                                                current_color.red = *red as u8;
                                                current_color.green = *green as u8;
                                                current_color.blue = *blue as u8;
                                            },
                                            [OscType::Double(red), OscType::Double(green), OscType::Double(blue)] => {
                                                current_color.red = *red as u8;
                                                current_color.green = *green as u8;
                                                current_color.blue = *blue as u8;
                                            },
                                            [OscType::Color(color)] => {
                                                current_color.red = color.red;
                                                current_color.green = color.green;
                                                current_color.blue = color.blue;
                                            },
                                            _ => {
                                                eprintln!("Unexpected OSC /color command: {:?}", msg.args);
                                            }
                                        }

                                        set_output(&mut current_output, *current_color).unwrap();
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

fn set_output(output: &mut Output, color: Color) -> rppal::gpio::Result<()> {
    output.red_pin.set_pwm_frequency(output.frequency, color.red as f64 / 255.0)?;
    output.green_pin.set_pwm_frequency(output.frequency, color.green as f64 / 255.0)?;
    output.blue_pin.set_pwm_frequency(output.frequency, color.blue as f64 / 255.0)?;

    Ok(())
}

fn main() {
    let initial = Color { red: 242, green: 155, blue: 212 };

    let gpio = Gpio::new().unwrap();

    let mut output = Output {
        frequency: 60.0,
        red_pin: gpio.get(17).unwrap().into_output(),
        green_pin: gpio.get(27).unwrap().into_output(),
        blue_pin: gpio.get(22).unwrap().into_output(),
    };

    set_output(&mut output, initial).unwrap();

    let current_color = Arc::new(Mutex::new(initial.clone()));
    let rocket_color = Arc::clone(&current_color);
    let osc_color = Arc::clone(&current_color);

    let current_output = Arc::new(Mutex::new(output));
    let rocket_output = Arc::clone(&current_output);
    let osc_output = Arc::clone(&current_output);

    rocket::ignite()
        .mount("/", routes![get_color, set_color, files, form, form_submit])
        .register(catchers![bad_request, unprocessable_entity, not_found])
        .manage(rocket_color)
        .manage(rocket_output)
        .attach(Template::fairing())
        .attach(AdHoc::on_launch("OSC Server", |_rocket| {
            thread::spawn(move || {
                osc_server(osc_color, osc_output);
            });
        }))
        .launch();
}
