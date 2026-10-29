//! Route: the name table. Payload decode, one handler per name, the reply frame.

use crate::ipc::contract::call;
use crate::ipc::contract::{ErrorCode as Code, Frame, Viewer};
use crate::server::api::{Response, call as call_group, post as post_group, subscribe};

/// Decode `payload` as `T`, or answer `bad_payload`.
macro_rules! decode {
    ($frame:expr, $t:ty) => {
        match serde_json::from_value::<$t>($frame.payload.clone()) {
            Ok(v) => v,
            Err(e) => {
                return Response::Frame(Frame::error_to(
                    $frame,
                    Code::BadPayload,
                    &format!("{}: {e}", $frame.name),
                ));
            }
        }
    };
}

/// Wrap a handler result; any error becomes `busy` when marked, else `internal`.
macro_rules! answer {
    ($frame:expr, $result:expr) => {{
        let frame = $frame;
        match $result {
            Ok(value) => match serde_json::to_value(value) {
                Ok(payload) => Response::Frame(Frame::reply_to(&frame, payload)),
                Err(e) => {
                    Response::Frame(Frame::error_to(&frame, Code::Internal, &format!("{e:#}")))
                }
            },
            Err(e) => Response::Frame(Frame::error_to(&frame, Code::Internal, &format!("{e:#}"))),
        }
    }};
}

pub fn call(cfg: &crate::server::config::Config, viewer: &Viewer, frame: Frame) -> Response {
    match frame.name.as_str() {
        "init" => {
            let ask = decode!(&frame, call::init::Ask);
            answer!(frame, call_group::init(cfg, viewer, ask.session, ask.limit))
        }
        "blocks" => {
            let ask = decode!(&frame, call::blocks::Ask);
            answer!(
                frame,
                call_group::blocks(cfg, viewer, ask.session, ask.ranges)
            )
        }
        "meta" => {
            let ask = decode!(&frame, call::meta::Ask);
            answer!(frame, call_group::meta(cfg, viewer, ask.session))
        }
        "usage" => {
            let ask = decode!(&frame, call::usage::Ask);
            answer!(frame, call_group::usage(cfg, viewer, ask.session, ask.lang))
        }
        "todo" => {
            let ask = decode!(&frame, call::todo::Ask);
            answer!(frame, call_group::todo(cfg, viewer, ask.session))
        }
        "memo" => {
            let ask = decode!(&frame, call::memo::Ask);
            answer!(
                frame,
                call_group::memo(cfg, viewer, ask.session, ask.op, ask.scope, ask.name)
            )
        }
        "children" => {
            let ask = decode!(&frame, call::children::Ask);
            answer!(frame, call_group::children(cfg, viewer, ask.session))
        }
        "config" => {
            let ask = decode!(&frame, call::config::Ask);
            answer!(frame, call_group::config(cfg, viewer, &ask.key))
        }
        "pools" => {
            let _ask = decode!(&frame, call::pools::Ask);
            answer!(frame, call_group::pools(cfg, viewer))
        }
        "sessions" => {
            let ask = decode!(&frame, call::sessions::Ask);
            answer!(frame, call_group::sessions(cfg, viewer, ask.under))
        }
        _ => unknown(frame),
    }
}

pub fn post(cfg: &crate::server::config::Config, viewer: &Viewer, frame: Frame) -> Response {
    let result = match frame.name.as_str() {
        "init" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Init);
            post_group::init(cfg, viewer, ask)
        }
        "name" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Name);
            post_group::name(cfg, viewer, ask)
        }
        "cwd" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Cwd);
            post_group::cwd(cfg, viewer, ask)
        }
        "switch" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Switch);
            post_group::switch(cfg, viewer, ask)
        }
        "profile" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Profile);
            post_group::profile(cfg, viewer, ask)
        }
        "todo_on" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Todo);
            post_group::todo_on(cfg, viewer, ask)
        }
        "todo_off" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Todo);
            post_group::todo_off(cfg, viewer, ask)
        }
        "delete" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Session);
            post_group::delete(cfg, viewer, ask)
        }
        "interrupt" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Session);
            post_group::interrupt(cfg, viewer, ask)
        }
        "rewind" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Rewind);
            post_group::rewind(cfg, viewer, ask)
        }
        "unwatch" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::Session);
            post_group::unwatch(cfg, viewer, ask)
        }
        "default_model" => {
            let ask = decode!(&frame, crate::ipc::contract::post::ask::DefaultModel);
            post_group::default_model(cfg, viewer, ask)
        }
        "reload" => post_group::reload(cfg, viewer),
        _ => return unknown(frame),
    };
    match result {
        Ok(payload) => Response::Frame(Frame::reply_to(&frame, payload)),
        Err(e) => Response::Frame(Frame::error_to(&frame, Code::Internal, &format!("{e:#}"))),
    }
}

pub fn subscribe(cfg: &crate::server::config::Config, viewer: &Viewer, frame: Frame) -> Response {
    match frame.name.as_str() {
        "watch" => {
            let ask = decode!(&frame, crate::ipc::contract::subscribe::watch);
            match subscribe::watch(cfg, viewer, ask.session) {
                Ok(rx) => Response::Stream(rx),
                Err(e) => {
                    Response::Frame(Frame::error_to(&frame, Code::Internal, &format!("{e:#}")))
                }
            }
        }
        "submit" => {
            let ask = decode!(&frame, crate::ipc::contract::subscribe::submit);
            match subscribe::submit(
                cfg,
                viewer,
                ask.session,
                ask.prefix.as_deref(),
                &ask.text,
                ask.suffix.as_deref(),
            ) {
                Ok((_, Some(turn))) => {
                    Response::Frame(Frame::reply_to(&frame, serde_json::json!({ "turn": turn })))
                }
                Ok((_, None)) => busy(&frame),
                Err(e) => {
                    Response::Frame(Frame::error_to(&frame, Code::Internal, &format!("{e:#}")))
                }
            }
        }
        "compact" => {
            let ask = decode!(&frame, crate::ipc::contract::subscribe::compact);
            match subscribe::compact(cfg, viewer, ask.session, ask.retention, ask.focus) {
                Ok((_, Some(turn))) => {
                    Response::Frame(Frame::reply_to(&frame, serde_json::json!({ "turn": turn })))
                }
                Ok((_, None)) => busy(&frame),
                Err(e) => {
                    Response::Frame(Frame::error_to(&frame, Code::Internal, &format!("{e:#}")))
                }
            }
        }
        _ => unknown(frame),
    }
}

fn busy(frame: &Frame) -> Response {
    Response::Frame(Frame::error_to(
        frame,
        Code::Busy,
        "the session is already in a round",
    ))
}

fn unknown(frame: Frame) -> Response {
    Response::Frame(Frame::error_to(
        &frame,
        Code::UnknownName,
        &format!("no such name: {}", frame.name),
    ))
}
