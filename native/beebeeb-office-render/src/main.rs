//! The Office helper's executable. It takes no argument, reads no environment variable and opens no file: the
//! sandbox hands it two pipes, and [`serve`] answers on them until the app closes its end.

use beebeeb_office_render::{Granted, Office, serve, start};

fn main() {
    start();
    let code = serve(
        Box::new(std::io::stdin()),
        &mut std::io::stdout().lock(),
        &Granted,
        &Office,
    );
    std::process::exit(code.into())
}
