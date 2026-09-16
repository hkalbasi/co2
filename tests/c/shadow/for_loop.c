//@ mode: c
//@ run-status: 1

#include <stdlib.h>
#include <stddef.h>

struct S { int x; struct S *prev; };

struct S *mk(void) {
    return malloc(sizeof(struct S));
}

void use(struct S *p) {}

int main(void) {
  struct S *i = mk();
  for (struct S *i = mk(); i; i = i->prev)
    use(i);
  i->x = 1;
  return i->x;
}
