//! `[notes]`, the scripted test service. `POST [notes]/run` and `GET [notes]/run` run
//! the script in their input, step by step, so a test states what a handler
//! does as data:
//!
//!   {"insert": "a"}                                insert a note, its id drawn from the op
//!   {"replace": ["a", "b"]}                        change note a's text to b, or fail with a 404
//!   {"set": ["k", "b"]}                            set the text of the note keyed k to b, looking it up
//!                                                  by its key and inserting it when there is none
//!   {"delete": "a"}                                delete note a, or fail with a 404
//!   {"unless": "a", "do": [..]}                    run the steps in `do` only when no note says a
//!   {"read": true}                                 add the texts this handler sees to the output
//!   {"call": "POST [notes]/run", "input": {..}}    make a call, adding its answer to the output
//!   {"call": .., "catch": true}                    the same, adding a failure's status instead of failing
//!   {"fail": "why"}                                fail with a 409
//!   {"hang": true}                                 never settle, keeping the call for `hung`
//!
//! `GET [notes]/hang` never settles either. `[notes]` must settle within
//! `NOTES_TIMEOUT`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Result, conflict, not_found, unknown};
use crate::{Carbon, Db, HandlerContext, Service};

// `[notes]`'s timeout, unlike the default, so a test sees its own set a call's deadline.
pub const NOTES_TIMEOUT: Duration = Duration::from_millis(100);

thread_local! {
  // The calls `[notes]` left hanging, for a test to settle late.
  static HUNG: RefCell<Vec<Context>> = RefCell::default();
}

// The calls `[notes]` has left hanging since this was last asked.
pub fn hung() -> Vec<Context> {
  HUNG.with(|hung| hung.take())
}

fn hang(context: Context) {
  HUNG.with(|hung| hung.borrow_mut().push(context));
}

pub fn register_notes(carbon: &Rc<Carbon>) {
  register_script(carbon, "notes");
}

// The scripted test service under a name of its own, with its own notes.
pub fn register_script(carbon: &Rc<Carbon>, name: &str) {
  register_script_making(carbon, name, &[]);
}

// The scripted test service, declaring the effects it makes again on every run.
pub fn register_script_making(carbon: &Rc<Carbon>, name: &str, always: &[&str]) {
  let setup = Rc::new(|db: Rc<dyn Db>| db.run("CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID", &[]).map(|_| ()));
  let service = Service::replicated(name, &format!("{name}-v1"), Some("db"), NOTES_TIMEOUT, setup, Rc::new(notes));
  carbon.register(Service { always: always.iter().map(|effect| effect.to_string()).collect(), ..service }).unwrap();
}

pub fn texts(db: &dyn Db) -> Result<Value> {
  let rows = db.all("SELECT text FROM note ORDER BY text", &[])?;
  Ok(Value::Array(rows.into_iter().map(|row| row["text"].clone()).collect()))
}

pub type Context = Rc<dyn HandlerContext>;

fn notes(context: Context) {
  match context.resource() {
    "/run" => {
      let script = context.input()["do"].as_array().cloned().unwrap_or_default();
      step(context, VecDeque::from(script), vec![])
    }
    "/texts" => {
      let result = seen(context.as_ref());
      let _ = context.settle(result);
    }
    "/hang" => hang(context),
    resource => {
      let _ = context.settle(Err(unknown(context.method(), resource)));
    }
  }
}

// The texts a handler sees, by the plain name its service gives the table.
fn seen(db: &dyn Db) -> Result<Value> {
  let rows = db.all("SELECT text FROM note ORDER BY text", &[])?;
  Ok(Value::Array(rows.into_iter().map(|row| row["text"].clone()).collect()))
}

// A step that changes a note, which fails when there is no such note.
fn changed(rows: Result<u64>, text: &Value) -> Result<()> {
  match rows? {
    0 => Err(not_found(format!("No note {text}"))),
    _ => Ok(()),
  }
}

// Set a note's text by its key, as an edit looks up the one row it changes.
fn set(db: &dyn Db, key: &Value, text: &Value) -> Result<()> {
  let found = db.all("SELECT id FROM note WHERE id = ?", std::slice::from_ref(key))?;
  let sql = if found.is_empty() { "INSERT INTO note (text, id) VALUES (?, ?)" } else { "UPDATE note SET text = ? WHERE id = ?" };
  db.run(sql, &[text.clone(), key.clone()]).map(|_| ())
}

// Run the rest of a script, then settle with what it gathered.
fn step(context: Context, mut script: VecDeque<Value>, mut output: Vec<Value>) {
  let Some(next) = script.pop_front() else {
    let _ = context.settle(Ok(json!(output)));
    return;
  };
  let done = if let Some(text) = next.get("insert") {
    // A GET has no op to draw an id from, and its `run` refuses before the id matters.
    let id = context.op().map(|op| op.id()).unwrap_or_default();
    context.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!(id), text.clone()]).map(|_| ())
  } else if let Some([from, to]) = next["replace"].as_array().map(Vec::as_slice) {
    changed(context.run("UPDATE note SET text = ? WHERE text = ?", &[to.clone(), from.clone()]), from)
  } else if let Some([key, text]) = next["set"].as_array().map(Vec::as_slice) {
    set(context.as_ref(), key, text)
  } else if let Some(text) = next.get("delete") {
    changed(context.run("DELETE FROM note WHERE text = ?", std::slice::from_ref(text)), text)
  } else if let Some(text) = next.get("unless") {
    let steps = next["do"].as_array().cloned().unwrap_or_default();
    seen(context.as_ref()).map(|texts| {
      if !texts.as_array().is_some_and(|texts| texts.contains(text)) {
        steps.into_iter().rev().for_each(|step| script.push_front(step));
      }
    })
  } else if next.get("read").is_some() {
    seen(context.as_ref()).map(|texts| output.push(texts))
  } else if let Some(why) = next["fail"].as_str() {
    Err(conflict(why))
  } else if next.get("hang").is_some() {
    return hang(context);
  } else {
    let catch = next["catch"] == json!(true);
    let caller = Rc::clone(&context);
    let key = next["call"].as_str().unwrap().to_string();
    return context.call(&key, next["input"].clone(), Box::new(move |result| match result {
      Ok(value) => {
        output.push(value);
        step(caller, script, output)
      }
      Err(error) if catch => {
        output.push(json!({"error": error.status}));
        step(caller, script, output)
      }
      Err(error) => {
        let _ = caller.settle(Err(error));
      }
    }));
  };
  match done {
    Ok(()) => step(context, script, output),
    Err(error) => {
      let _ = context.settle(Err(error));
    }
  }
}
