#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::Mutex;

use rocket::State;
use rocket::http::Status;

use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

use rosc::{OscPacket, OscType};

use rppal::gpio::{Gpio, OutputPin};

use serde::{Serialize, Deserialize};


#[derive(Clone, Copy, Serialize, Deserialize)]
struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Clone, Serialize, Deserialize)]
struct Error {
    status: String,
    message: String,
}

type CurrentColor = Mutex<Color>;

struct Output {
    frequency: f64,
    red_pin: OutputPin,
    green_pin: OutputPin,
    blue_pin: OutputPin,
}

type CurrentOutput = Mutex<Output>;

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

#[get("/")]
fn form() -> Template {
    Template::render("form", HashMap::<String, String>::new())
}

#[post("/")]
fn form_submit() -> Template {
    form()
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

fn osc_server(output: &CurrentOutput) {
    let socket = UdpSocket::bind("127.0.0.1:1337");

    let mut buffer = [0u8; rosc::decoder::MTU];

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((size, addr)) => {
                match rosc::decoder::decode(&buffer[..size]) {
                    Ok(packet) => {
                        match packet {
                            OscPacket::Message(msg) => {
                                match msg.addr.as_ref() {
                                    "/color" => {
                                        match msg.args[..] {
                                            [OscType::Color(color)] => {
                                                let mut current_output = output.lock().unwrap();
                                                set_output(&mut current_output, Color { red: color.red, green: color.green, blue: color.blue }).unwrap();
                                            },
                                            _ => {
                                                eprintln!("Unexpected OSC /color command: {:?}", msg.args);
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
                    Err(err) {
                        eprintln!("Error decoding OSC packet: {}", err);
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

    rocket::ignite()
        .mount("/", routes![get_color, set_color, form, form_submit])
        .register(catchers![bad_request, unprocessable_entity, not_found])
        .manage(Mutex::new(initial.clone()))
        .manage(Mutex::new(output))
        .attach(Template::fairing())
        .attach(AdHoc::on_launch("OSC Server", |rocket| {
            thread::spawn(|| {
                osc_server(rocket.state::<CurrentOutput>().unwrap());
            });
        }))
        .launch();
}
