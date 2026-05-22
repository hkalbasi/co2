//@ mode: c
//@ run-status: 0
/* Minimized: #ifdef inside return expression with && */

struct s { int a; int b; };

static int check(const struct s *p, const struct s *q)
{
    return (p->a &&
#ifdef FOO
        (p->a < q->a)
#else
        p->a <= q->a
#endif
    );
}

int main(void) { 
    struct s s = { 2, 4 };
    if (!check(&s, &s)) {
        return 1;
    }
    return 0;
}
