static __thread int tls_var = 42;

int get_tls(void) { return tls_var; }
void set_tls(int v) { tls_var = v; }
