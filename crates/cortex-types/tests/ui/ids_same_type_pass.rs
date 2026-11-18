// F1-04 pass partner: a ProjectId goes where a ProjectId is expected.
use cortex_types::ProjectId;

fn wants_project(_: ProjectId) {}

fn main() {
    let _pass_project = |p: ProjectId| wants_project(p);
}
