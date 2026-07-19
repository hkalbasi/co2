//@ mode: c
//@ run-status: 0

int f1();
int f1();
extern int f1();
int f1();
int f1() {
    return 5;
}
int f1();
extern int f1();

int f2();
extern int f2();

typedef int t1; 
typedef int t1;

typedef t1 t2;
typedef int t2;

static int f3();
int f3();
static int f3() {
    return 7;
}

static int s1;
static int s1 = 3;
static int s1;
extern int s1;
extern int s1;
static int s1;

extern int s2;
int s2;
int s2 = 4;
int s2;
extern int s2;

typedef int func_ty();
int f4();
func_ty f4;
extern func_ty f4;
int f4() {
    return 4;
}

func_ty f5;
int f5() {
    return 5;
}

int f6();
typeof(f5) f6;
extern typeof(f5) f6;
int f6() {
    return 6;
}

typeof(f5) f7;
int f7() {
    return 7;
}

int main() {
    if (f1() != 5) {
        return 1;
    }
    if (f3() != 7) {
        return 3;
    }
    if (s1 != 3) {
        return 1;
    }
    if (s2 != 4) {
        return 2;
    }
    if (f4() != 4) {
        return 4;
    }
    if (f5() != 5) {
        return 5;
    }
    if (f6() != 6) {
        return 6;
    }
    if (f7() != 7) {
        return 7;
    }
    return 0;
}
