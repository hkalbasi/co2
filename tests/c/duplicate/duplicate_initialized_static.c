//@ mode: c
//@ compile-fail

int s1 = 4;
int s1 = 4;
  //^^ the name `s1` is defined multiple times

int main() {
}
