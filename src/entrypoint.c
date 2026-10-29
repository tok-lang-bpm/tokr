// Routine registration is done by the Rust library; this file forwards to it so that the
// linker keeps the static library.

void R_init_tokr_extendr(void *dll);
void register_extendr_panic_hook(void);

void R_init_tokr(void *dll) {
    register_extendr_panic_hook();
    R_init_tokr_extendr(dll);
}
