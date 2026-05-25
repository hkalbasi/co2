//@ mode: c
//@ compile-fail

  #error "bad config"
// ^^^^^ error: #error "bad config"

int main(void) {
    return 0;
}
