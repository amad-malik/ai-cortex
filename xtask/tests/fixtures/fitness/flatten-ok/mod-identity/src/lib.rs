
#[derive(Debug)]
pub struct Other { pub a: Vec<Option<u8>> }
pub fn nested(o: &Other) -> usize { o.a.iter().flatten().count() }
// another derive crate (clap) has its own flatten attribute: it is not serde's
pub struct Cli {
    #[command(flatten)]
    pub args: Args,
}
