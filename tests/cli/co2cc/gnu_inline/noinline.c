static __attribute__((noinline)) int target(int x) { return x * 2 + 1; }
int main(int argc, char **argv) {
  (void)argv;
  return target(argc) != 3;
}
