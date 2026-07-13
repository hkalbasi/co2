//@ mode: c
//@ compile-fail

typedef struct {
  union {

    struct {
      void *_lower;
      void *_upper;
    } _addr_bnd;

    unknown_type _pkey;
  //^^^^^^^^^^^^ error: unresolved name unknown_type
  } _bounds;
} _sigfault;

int main(void) { return 0; }
