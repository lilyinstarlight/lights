#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

use std::collections::HashMap;
use std::sync::Mutex;

use rocket::State;
use rocket::http::Status;

use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

use serde::{Serialize, Deserialize};

use rppal::gpio::Gpio;
use rppal::gpio::OutputPin;


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
        .launch();
}
