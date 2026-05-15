//@ mode: c
//@ run-status: 0

#include <stdarg.h>

int simple_varargs(int foo, ...) {
    return foo;
}

int implicit_varargs() {
    return 12;
}

struct s7 { char x[7]; } s7 = { "lmnopqr" };

int multiple_types(int normal_arg, ...) {
    if (normal_arg != 5) {
        return 10;
    }

    va_list args;
    va_start(args, normal_arg);

    if (va_arg(args, int) != 1) {
        return 1;
    }

    if (*va_arg(args, int *) != 1) {
        return 2;
    }

    struct s7 s = va_arg(args, struct s7);
    if (s.x[2] != 'n') {
        return 3;
    }

    va_end(args);

    return 0;
}

int helper_fn(int n, va_list ap) {
	int total = 0;
	int i;
	for (i = 0; i < n; i += 1) {
		total += va_arg(ap, int);
	}
	return total;
}

int sum_using_helper_fn(int n, ...) {
	va_list ap;
	va_start(ap, n);
	int total = helper_fn(n, ap);
	va_end(ap);
	return total;
}

int read_one(va_list *ap) {
	return va_arg(*ap, int);
}

int reuse(int n, ...) {
	va_list ap;
	va_start(ap, n);
	if (read_one(&ap) != 1) {
		va_end(ap);
		return 1;
	}
	va_end(ap);
	va_start(ap, n);
	if (read_one(&ap) != 1) {
		va_end(ap);
		return 2;
	}
    if (read_one(&ap) != 2) {
		va_end(ap);
		return 3;
	}
	va_end(ap);
	return 0;
}

struct VarargHolder { void (*f)(char *, va_list); };
void f1(char *s, va_list ap) {}
int fill_vararg_holder(struct VarargHolder *p) { p->f = f1; return 0; }


int main() {
    if (simple_varargs(5, 2, "salam") != 5) {
        return 1;
    }
    if (implicit_varargs(5, 2, "salam") != 12) {
        return 2;
    }
    int p = 1;
    if (multiple_types(5, p, &p, s7)) {
        return 3;
    }
    if (sum_using_helper_fn(3, 1, 2, 4) != 7) {
        return 4;
    }
    if (reuse(2, 1, 2)) {
        return 50 + reuse(2, 1, 2);
    }

    struct VarargHolder va_holder = { .f = 0 };
    fill_vararg_holder(&va_holder);
    if (va_holder.f != f1) {
        return 5;
    }

    return 0;
}
