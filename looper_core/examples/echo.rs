use looper_core::{Child, Core, ObjectId};
use std::io::Read;
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
        if let State::WaitingForRoutes(ref mut r) = self.state {
            let mut s = String::new();
            r.stdout.read_to_string(&mut s).unwrap();
            let echo = core
                .spawn(Command::new("echo").arg(s))
                .expect("echo executable must exist.");
            core.register_reaper(&echo, self.id, Sequence::handle_echo_death);
            self.state = State::WaitingForEcho(echo);
        } else {
            unreachable!()
        }
    }

    fn handle_echo_death(&mut self, core: &mut Core) {
        if let State::WaitingForEcho(ref mut e) = self.state {
            let mut s = String::new();
            e.stdout.read_to_string(&mut s).unwrap();
            eprintln!("Got echo output: {}", s);
            self.state = State::Terminating;
            core.exit();
        } else {
            unreachable!()
        }
    }
}

fn main() {
    let mut core = Core::new();
    let route = core
        .spawn(Command::new("route"))
        .expect("route executable must exist.");
    let id = core.next_id();
    core.register_reaper(&route, id, Sequence::handle_route_death);
    core.add(Sequence {
        state: State::WaitingForRoutes(route),
        id,
    });
    core.run();
}
