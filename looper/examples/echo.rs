use looper::{Child, Core, ObjectId};
use std::io::{Read, Write};
use std::process::Command;

// Tests running commands in sequence.
// First run the "route" command and capture it's output. Then run echo after route has terminated.

struct Sequence {
    id: ObjectId,
    state: State,
}

enum State {
    WaitingForRoutes(Child),
    WaitingForEcho(Child),
    Terminating,
}

impl Sequence {
    fn handle_route_death(&mut self, core: &mut Core) {
        eprintln!("handling route death");
        if let State::WaitingForRoutes(ref mut r) = self.state {
            let mut s = String::from("Happy happy.");
            r.stdout.read_to_string(&mut s).unwrap();
            eprintln!("{}", s);
            let mut echo = core
                .spawn(Command::new("echo"))
                .expect("echo executable must exist.");
            core.register_reaper(&echo, self.id, Sequence::handle_echo_death);
            echo.stdin.write(&s.into_bytes()).unwrap();
            echo.stdin.
            self.state = State::WaitingForEcho(echo);
        } else {
            unreachable!()
        }
    }

    fn handle_echo_death(&mut self, core: &mut Core) {
        eprintln!("handling echo death");
        if let State::WaitingForEcho(ref mut e) = self.state {
            let mut s = String::new();
            e.stdout.read_to_string(&mut s).unwrap();
            eprintln!("Got echo output: {}", s);
            self.state = State::Terminating;
            core.exit();
            eprintln!("called exit.");
        } else {
            unreachable!()
        }
    }
}

fn main() {
    let mut core = Core::new();
    let route = core
        .spawn(Command::new("route").arg("print"))
        .expect("route executable must exist.");
    let id = core.next_id();
    core.register_reaper(&route, id, Sequence::handle_route_death);
    core.add(Sequence {
        state: State::WaitingForRoutes(route),
        id,
    });
    core.run();
}
