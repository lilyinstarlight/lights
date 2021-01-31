#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;

use std::collections::HashMap;
use std::sync::Mutex;

use rocket::State;
use rocket::http::Status;

use rocket_contrib::json::Json;
use rocket_contrib::templates::Template;

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

#[get("/color")]
fn get_color(current: State<CurrentColor>) -> Json<Color> {
    Json(*current.lock().unwrap())
}

#[put("/color", data = "<color>")]
fn set_color(color: Json<Color>, current: State<CurrentColor>) -> Status {
    let mut current_color = current.lock().unwrap();

    current_color.red = color.red;
    current_color.green = color.green;
    current_color.blue = color.blue;

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

#[catch(404)]
fn not_found() -> Json<Error> {
    Json(Error {
        status: String::from("error"),
        message: String::from("Resource not found"),
    })
}

fn main() {
    rocket::ignite()
        .mount("/", routes![get_color, set_color, form])
        .register(catchers![bad_request, not_found])
        .manage(Mutex::new(Color { red: 242, green: 155, blue: 212, }))
        .attach(Template::fairing())
        .launch();
}
