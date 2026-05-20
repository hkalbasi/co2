//@ mode: c
//@ run-status: 0

#include <stddef.h>
#include <stdint.h>

int main0() {
	return 0;
}

int main1() {
	int x;
	int *p;
	int **pp;

	x = 0;
	p = &x;
	pp = &p;

	if(*p)
		return 1;
	if(**pp)
		return 1;
	else
		**pp = 1;

	if(x)
		return 0;
	else
		return 1;
}

int main2()
{
	int x;
	x = 4;
	return x - 4;
}

int main3() {
	int x;
	
	x = 1;
	for(x = 10; x; x = x - 1)
		;
	if(x)
		return 1;
	x = 10;
	for (;x;)
		x = x - 1;
	if(x)
		return 1;
	x = 10;
	while (x)
		x = x - 1;
	if(x)
		return 1;
	x = 10;
	do 
		x = x - 1;
	while(x);
	if(x)
		return 1;
	return 0;
}

int main4()
{
	int x;
	
	x = 1;
	x = x * 10;
	x = x / 2;
	x = x % 3;
	return x - 2;
}

int main5()
{
	start:
		goto next;
		return 1;
	success:
		return 0;
	next:
	foo:
		goto success;
		return 1;
}

int main6()
{
	int x;
	int *p;
	int *q;
	int n;

	x = 7;
	n = 0;
	p = &x;
	q = p + n;
	if (*q - 7)
		return 1;
	q = p - n;
	if (*q - 7)
		return 1;
	p[0] = 9;
	if (*p - 9)
		return 1;
	return 0;
}

int main7()
{
	int arr[4];
	int *p;

	arr[0] = 1;
	arr[1] = 2;
	p = arr;
	if (*(arr + 1) - 2)
		return 1;

	if (&arr[1] - &arr[0] != 1)
		return 1;

	*(p + 2) = 5;
	p[3] = 7;

	if (arr[2] - 5)
		return 1;
	if (*(arr + 3) - 7)
		return 1;

	return arr[0] + arr[1] + arr[2] + arr[3] - 15;
}

int main8()
{
	struct { int x; int y; } s, s2, *s3;
	
	s.x = 3;
	s.y = 5;
	if (s.y - s.x - 2) {
		return 1;
	}
	s2 = s;
	if (s2.y - s2.x - 2) {
		return 2;
	}
	s3 = &s2;
	if (s3->y - s3->x - 2) {
		return 2;
	}
	return 0;
}

int main9()
{
	int a;
	int b;
	int c;
	int x;

	a = 20;
	b = 6;
	c = 3;

	if (a + b - 26)
		return 1;
	if (a - b - 14)
		return 1;
	if (a * c - 60)
		return 1;
	if (a / b - 3)
		return 1;
	if (a % b - 2)
		return 1;
	if ((a | b) - 22)
		return 1;
	if ((a ^ b) - 18)
		return 1;
	if ((a & b) - 4)
		return 1;
	if ((a << 1) - 40)
		return 1;
	if ((a >> 2) - 5)
		return 1;

	if ((a == 20) - 1)
		return 1;
	if ((a != 20) - 0)
		return 1;
	if ((a < 21) - 1)
		return 1;
	if ((a <= 20) - 1)
		return 1;
	if ((a > 19) - 1)
		return 1;
	if ((a >= 20) - 1)
		return 1;
	if ((!0) - 1)
		return 1;
	if ((!a) - 0)
		return 1;
	if ((~5) + 6)
		return 1;
	if ((-a) + 20)
		return 1;

	x = 0;
	if (0 && (x = 1))
		return 1;
	if (x)
		return 1;
	if ((1 || (x = 2)) - 1)
		return 1;
	if (x)
		return 1;
	if ((0 || (x = 3)) - 1)
		return 1;
	if (x - 3)
		return 1;
	if ((1 && (x = 4)) - 1)
		return 1;
	if (x - 4)
		return 1;
	if ((x++, 3) != 3)
		return 1;
	if (x - 5)
		return 1;

	return 0;
}

int main10()
{
	int arr[3];
	int *p;
	int *q;
	int x;
	int y;

	arr[0] = 10;
	arr[1] = 20;
	arr[2] = 30;

	p = arr;
	q = ++p;
	if (*q - 20)
		return 1;
	if (*p - 20)
		return 1;

	q = p++;
	if (*q - 20)
		return 1;
	if (*p - 30)
		return 1;

	q = --p;
	if (*q - 20)
		return 1;
	if (*p - 20)
		return 1;

	q = p--;
	if (*q - 20)
		return 1;
	if (*p - 10)
		return 1;

	x = 4;
	y = ++x;
	if (x - 5 || y - 5)
		return 1;
	y = x++;
	if (x - 6 || y - 5)
		return 1;

	return 0;
}

int main11()
{
	int i;
	int sum;

	sum = 0;
	for (i = 0; i < 10; i++) {
		if (i == 2)
			continue;
		if (i == 7)
			break;
		sum = sum + i;
	}
	if (sum - 19)
		return 1;

	i = 0;
	sum = 0;
	while (1) {
		i++;
		if (i == 2)
			continue;
		if (i == 5)
			break;
		sum = sum + i;
	}
	if (sum - 8)
		return 1;

	i = 0;
	sum = 0;
	do {
		i++;
		if (i == 2)
			continue;
		if (i == 5)
			break;
		sum = sum + i;
	} while (1);
	if (sum - 8)
		return 1;

	return 0;
}

int main12()
{
	int a;
	int arr[3];
	int *p;

	a = 10;
	a += 2;
	if (a - 12)
		return 1;
	a -= 3;
	if (a - 9)
		return 1;
	a *= 4;
	if (a - 36)
		return 1;
	a /= 6;
	if (a - 6)
		return 1;
	a %= 4;
	if (a - 2)
		return 1;
	a <<= 3;
	if (a - 16)
		return 1;
	a >>= 2;
	if (a - 4)
		return 1;
	a |= 3;
	if (a - 7)
		return 1;
	a &= 6;
	if (a - 6)
		return 1;
	a ^= 5;
	if (a - 3)
		return 1;

	arr[0] = 11;
	arr[1] = 22;
	arr[2] = 33;
	p = arr;
	p += 1;
	if (*p - 22)
		return 1;
	p -= 1;
	if (*p - 11)
		return 1;

	return 0;
}

int main13()
{
	int v;
	int x;
	int i;
	int sum;

	x = 0;
	v = 2;
	switch (v) {
	case 1:
		x = 10;
		break;
	case 2:
		x = 20;
		break;
	default:
		x = 30;
		break;
	}
	if (x - 20)
		return 1;

	x = 0;
	v = 1;
	switch (v) {
	case 1:
		x = x + 1;
	case 2:
		x = x + 2;
		break;
	default:
		x = x + 4;
		break;
	}
	if (x - 3)
		return 1;

	x = 0;
	v = 99;
	switch (v) {
	case 0:
		x = 1;
		break;
	default:
		x = 7;
		break;
	}
	if (x - 7)
		return 1;

	sum = 0;
	for (i = 0; i < 5; i++) {
		switch (i) {
		case 1:
			continue;
		case 3:
			break;
		default:
			sum = sum + i;
			break;
		}
	}
	/* i=0 adds 0; i=1 continue; i=2 adds 2; i=3 breaks switch only; i=4 adds 4 */
	if (sum - 6)
		return 1;

	return 0;
}

int main14()
{
	int x;
	int *p;
	void *p_void;
	int arr[4];
	struct { int a; char b; } s;

	x = 2;
	p = &x;
	p_void = p;
	if(*((int*)p_void) != 2)
		return 1;
	if (sizeof x - 4)
		return 1;
	if (sizeof(int) - 4)
		return 1;
	if (sizeof(int*) - 8)
		return 1;
	if (sizeof(p) - 8)
		return 1;
	if (sizeof(*p) - 4)
		return 1;
	if (sizeof arr - 16)
		return 1;
	if (sizeof(s) - 8)
		return 1;
	p = (int*)5;
	if (((long long int)p) - 5)
		return 1;
	if (((int*)5) != p)
		return 1;

	(void)*p;

	return 0;
}

int main15()
{
	int a[5] = { [2] = 7, [0] = 9, 1 };
	int b[5] = { [sizeof(char)] = 9, 1 };
	struct { int x; int y; int z; } s = { .z = 8, .x = 3 };
	struct { int x; int y; } p[2] = {
		[1] = { .y = 6, .x = 5 },
		[0] = { .x = 1, .y = 2 },
	};
	int m[2][3] = { [1][2] = 4, [0][1] = 3 };

	if (a[3] || a[4])
		return 1;
	if (a[0] - 9 || a[1] - 1 || a[2] - 7)
		return 1;
	if (b[1] - 9 || b[2] - 1)
		return 1;
	if (s.x - 3 || s.y || s.z - 8)
		return 1;
	if (p[0].x - 1 || p[0].y - 2 || p[1].x - 5 || p[1].y - 6)
		return 1;
	if (m[0][1] - 3 || m[1][2] - 4)
		return 1;
	if (m[0][0] || m[0][2] || m[1][0] || m[1][1])
		return 1;

	char x[4][3][3] = { [1][2] = "0", "1", "x2", {'3'}, {"4"} };
	if (x[1][2][0] != '0' || x[2][0][0] != '1' || x[2][1][1] != '2' 
		|| x[2][2][0] != '3' || x[3][0][0] != '4') {
		return 1;
	}

	char y[10][2] = { "overflowing", "1", "20" };
	if (y[0][1] != 'v' || y[1][1] != 0 || y[2][0] != '2') {
		return 1;
	}

	if (L'\0') {
		return 1;
	}

	return 0;
}

enum Color { RED, GREEN = 5, BLUE, WHITE = BLUE + 3 };

static int COLOR_TO_RGB[] = {
    [RED] = 255,
    [GREEN] = 255 * 256,
    [BLUE] = 255 * 256 * 256,
    [WHITE] = 256 * 256 * 256 - 1,
};

int main16()
{
	enum Color c;
	int x;

	int ar[] = { [WHITE] = 2 };

	c = GREEN;
	if (c) {
		c = BLUE;
	}
	if (c - 6)
		return 1;
	if (RED)
		return 1;
	if (GREEN - 5)
		return 1;
	if (WHITE - 9)
		return 1;
	if ((~WHITE) + 10)
		return 1;
	if ((~c) + 7)
		return 1;

	x = RED + GREEN + BLUE + WHITE;
	if (x - 20)
		return 1;

	ar[RED] = 4;

	if (ar[WHITE] != 2 || ar[RED] != 4) {
		return 1;
	}

	c = RED;
	if (ar[c] != 4) {
		return 1;
	}

	if (COLOR_TO_RGB[RED] + COLOR_TO_RGB[GREEN] + COLOR_TO_RGB[BLUE] != COLOR_TO_RGB[WHITE]) {
		return 1;
	}

	COLOR_TO_RGB[RED] = WHITE;
	if (ar[COLOR_TO_RGB[RED]] != 2) {
		return 1;
	}
	if (ar[COLOR_TO_RGB[c]] != 2) {
		return 1;
	}
	if (ar[COLOR_TO_RGB[(int)c]] != 2) {
		return 1;
	}

	return 0;
}

int main17()
{
	int a;
	long y;
	char *s;
	char *t;
	char *u;

	a = 'x';
	if (a - 120)
		return 1;
	a = '\n';
	if (a - 10)
		return 1;
	a = '\\';
	if (a - 92)
		return 1;
	a = '\'';
	if (a - 39)
		return 1;
	a = '\x93';
	if (a + 109)
		return 1;

	a = 2.5;
	if (a - 2)
		return 1;
	a = 0x10;
	if (a - 16)
		return 1;
	a = 010;
	if (a - 8)
		return 1;
	y = -1l;
	if (y + 1l)
		return 1;

	s = "foo" "bar";
	if (((int)s[0]) - 'f' || ((int)s[1]) - 'o' || ((int)s[2]) - 'o')
		return 1;
	if (((int)s[3]) - 'b' || ((int)s[4]) - 'a' || ((int)s[5]) - 'r')
		return 1;
	if (s[6])
		return 1;

	t = "a" "" "\x12";
	if (((int)t[0]) - 'a' || ((int)t[1]) - '\x12' || ((int)t[2]))
		return 1;
	u = "A\nB\t\\\"";
	if (((int)u[0]) - 'A' || ((int)u[1]) - '\n' || ((int)u[2]) - 'B')
		return 1;
	if (((int)u[3]) - '\t' || ((int)u[4]) - '\\' || ((int)u[5]) - '\"')
		return u[0] + 5;
	if (u[6])
		return u[0];

	u = "\x93";
	if (u[0] - '\x93' || u[1]) {
		return 1;
	}

	return 0;
}

int fp_id(int x)
{
	return x;
}

int (*fp_static)(int);
int (*fp_static2)(int) = 0;
int (*fp_static3)(int) = fp_id;
typedef struct { int (*f)(int); } fp_holder;

int main18()
{
	int (*fp)(int);

	fp = fp_id;

	if (fp(3) - 3)
		return 1;

	void* fp_void = &fp_id;
	fp = fp_void;

	if ((*******fp)(3) - 3)
		return 1;

	fp_holder s = { .f = fp_id };
	if (s.f(3) - 3)
		return 1;

	fp_holder s2;
	s2.f = s.f;
	if (s2.f(3) - 3)
		return 1;

	fp = 0;
	if (fp)
		return 1;

	if (fp_static)
		return 1;
	fp_static = s2.f;
	if (fp_static(3) - 3)
		return 1;

	if (fp_static2)
		return 1;
	fp_static2 = s2.f;
	if (fp_static2(3) - 3)
		return 1;

	if (fp_static3(4) - 4)
		return 1;
	fp_static3 = 0;
	if (fp_static3)
		return 1;
	
	if (0) {
		((void (*)(void))0) ();
	}

	return 0;
}

int main19()
{
	int x;
	int y;
	int z;

	x = 10;
	y = 20;
	z = 0;

	z = (x > y) ? x : y;
	if (z - 20)
		return 1;

	z = (x < y) ? x : y;
	if (z - 10)
		return 2;

	if ((x > y ? (x < y ? 1 : 2) : 3) - 3)
		return 3;
	if ((x < y ? (x > y ? 1 : 2) : 3) - 2)
		return 4;

	x = 0;
	if (x ? 1 : 0)
		return 5;
	x = 1;
	if (!x ? 1 : 0)
		return 6;

	return 0;
}

typedef int myint;
myint the_zero = (myint)0;

int zero()
{
	return the_zero;
}

struct S
{
	int (*zerofunc)();
} s = { &zero };

struct S * anon()
{
	return &s;
}

typedef struct S * (*fty)();

fty go()
{
	return &anon;
}

int main20()
{
	return go()()->zerofunc();
}

typedef struct S2 { int a; int b; } S2;

int main21()
{
	S2 *p, *p2;

	p = &(struct S2) { 1, 2 };
	if (p->a - 1)
		return 1;
	if (p->b - 2)
		return 1;

	p2 = &(S2) { 3, 4 };
	if (p2->a - 3)
		return 1;
	if (p2->b - 4)
		return 1;
	if (((S2) { .a=3 }).a != 3) {
		return 1;
	}

	return 0;
}

int main22() {
	;;;;;;;;;;;;
	return 0;
	return 1;
	return 2;
}

int counter = 0;
int uninit;

static inline int count() {
	counter += 1;
	return counter;
}

struct s_nested {
	int x;
	struct {
		int y;
		int z;
	} nest;
};

int main23()
{
	count();
	count();
	count();
	uninit = 4;
	return count() - uninit;
}

int main24()
{
	struct s_nested v;

	v.x = 1;
	v.nest.y = 2;
	v.nest.z = 3;
	if (v.x - 1)
		return 1;
	if (v.nest.y - 2)
		return 1;
	if (v.nest.z - 3)
		return 1;
	return 0;
}

typedef struct {
	int a;
	union {
		int b1;
		int b2;
	};
	struct { union { struct { int c; }; }; };
	struct {
		int d;
	};
} NestedStruct;

int main25()
{
	NestedStruct v;

	v.a = 1;
	v.b1 = 2;
	v.c = 3;
	v.d = 4;

	if (v.a != 1)
		return 1;
	if (v.b1 != 2 && v.b2 != 2)
		return 2;
	if (v.c != 3)
		return 3;
	if (v.d != 4)
		return 4;

	return 0;
}

struct S1_init {
	int a;
	int b;
};

struct S2_init {
	int a;
	int b;
	union {
		int c;
		int d;
	};
	struct S1_init s;
};

struct S2_init v_init = {1, 2, 3, {4, 5}};

int main26()
{
	if(v_init.a != 1)
		return 1;
	if(v_init.b != 2)
		return 2;
	if(v_init.c != 3 || v_init.d != 3)
		return 3;
	if(v_init.s.a != 4)
		return 4;
	if(v_init.s.b != 5)
		return 5;

	return 0;
}

int count_and_return_zero() {
	counter += 1;
	return 0;
}

int main27()
{
	counter = 0;
	switch(count_and_return_zero())
		case 0:
			;
	switch(count_and_return_zero())
		case 0:
			switch(count_and_return_zero()) {
				case 2:
				case 5:
					return 1;
				case 0:
					goto next27;
				default:
					return 1;
			}
	return 1;
	next27:
	switch(count_and_return_zero())
		case 1:
			return 1;
	switch(count_and_return_zero()) {
		{
			foo27:
			case 1:
				return 1;
		}
	}
	switch(count_and_return_zero()) {
		case 0:
			return counter - 6;
		case 1:
			return 1;
		default:
			return 1;
	}
}

int static_array[3] = {0, 1, 2};

int main28()
{
	if (static_array[0] != 0)
		return 1;
	if (static_array[1] != 1)
		return 2;
	if (static_array[2] != 2)
		return 3;
	
	return 0;
}

typedef int unsized_int_array[];
unsized_int_array static_ar1 = {10, 20, 3, 15, 1000, 60, 16};
char static_ar2[] = "foo bar foobar\x93";
int static_ar3[(1 + 2) * 2] = {0, 1, 2, 3, 4, 5};
typedef struct {
	char field1[2];
	int field2;
	int field3[2 * 2 + sizeof(int)];
	int field4[1024 / (8 * (int) sizeof (char))];
} complex_size;
int static_ar4[sizeof(complex_size)] = {5};
unsized_int_array static_ar5 = {[3] = 2, 4};
struct { int x, y; } static_ar6[] = {1, 2, 3};

int main29()
{
	int ar1[] = {1, 2, 3, 4, 5, 6};
	if (ar1[0] != 1 || ar1[5] != 6 || (sizeof(ar1) / sizeof(int) != 6)) {
		return 1;
	}

	unsized_int_array ar2 = {10, 20, 30};
	if (ar2[0] != 10 || ar2[2] != 30 || (sizeof(ar2) / sizeof(int) != 3)) {
		return 2;
	}

	char ar3[] = "Hello world\x94";
	if (ar3[0] != 'H' || ar3[10] != 'd' || ar3[11] != '\x94' || sizeof(ar3) != 13) {
		return 3;
	}

	int ar4[][5] = {1, 2, 3, 4, 5, 6, 7};

	if (ar4[1][1] != 7 || ar4[0][4] != 5 || sizeof(ar4) / sizeof(int) != 10) {
		return 3;
	}
	
	if (static_ar1[0] != 10 || static_ar1[6] != 16 || sizeof(static_ar1) / sizeof(int) != 7) {
		return 4;
	}

	if (static_ar2[0] != 'f' || static_ar2[14] != '\x93' || static_ar2[15] != 0 || sizeof(static_ar2) != 16) {
		return 5;
	}

	char* local_ar2 = static_ar2;
	if (local_ar2[0] != 'f' || local_ar2[15] != 0 || sizeof(local_ar2) != sizeof(char*)) {
		return 5;
	}

	int ar5[sizeof(complex_size) * 2];
	if (sizeof(ar5) / sizeof(int) != sizeof(complex_size) * 2) {
		return 6;
	}

	if (static_ar3[0] != 0 || static_ar3[5] != 5
		|| sizeof(static_ar3) / sizeof(int) != 6) {
		return 7;
	}

	if (static_ar4[0] != 5 || static_ar4[1] != 0
		|| sizeof(static_ar4) / sizeof(int) != sizeof(complex_size)) {
		return 8;
	}

	if (sizeof(static_ar5) / sizeof(int) != 5
		|| static_ar5[0] != 0 || static_ar5[2] != 0
		|| static_ar5[3] != 2 || static_ar5[4] != 4) {
		return 9;
	}

	if (sizeof(static_ar6) / sizeof(struct { int x, y; }) != 2
		|| static_ar6[0].x != 1 || static_ar6[0].y != 2 || static_ar6[1].x != 3) {
		return 10;
	}

	if (sizeof("hello") != 6) {
		return 11;
	}

	if (sizeof("\x93") != 2) {
		return 12;
	}

	return 0;
}

typedef struct { int x; int y; } inner_st;
typedef struct { inner_st x; int y; } middle_st;
typedef struct { int x; middle_st y; } outer_st;

int main30()
{
	int n1 = 1;
	middle_st s2 = { n1, n1, 3 };
	outer_st s3 = { 1, s2 };

	if (s3.y.x.x != 1) {
		return 1;
	}

	return 0;
}

int main31()
{
	short s = 1;
	long l = 1;

	s -= l;
	return s;
}

int count_inner_static() {
	static int x = 0;
	x += 1;
	return x;
}

int count_inner_static_array() {
	static int x[] = {0, 1, 2};
	x[0] += 10;
	x[1] += 20;
	return x[0] + x[1] + x[2];
}

int main32()
{
	count_inner_static();
	count_inner_static();
	if (count_inner_static() != 3) {
		return 1;
	}

	count_inner_static_array();
	count_inner_static_array();
	if (count_inner_static_array() != 93) {
		return 2;
	}
	return 0;
}

struct T static_s1;
struct T { int z; } static_s2;

int main33()
{
	struct T s0;
	s0.z = 100;
	struct T { int x; } s1;
	s1.x = 1;
	{
		struct T s4;
		s4.x = 3;
		struct T { int y; } s2;
		s2.y = 1;
		struct T s3;
		s3.y = 10;
		if (s0.z + s1.x + s2.y + s3.y + s4.x != 115)
			return 1;
	}
	struct T s3 = s1;
	if (s3.x != 1) {
		return 2;
	}
	return 0;
}

int declared_static;
int declared_static = 3;
int declared_static;

int declared_func();
int declared_func() {
	return 3;
}
int declared_func();
typedef int declared_func_ty();
declared_func_ty declared_func;

int main34() {
	int declared_func();
	return declared_static - declared_func();
}

int main35() {
	float x = 1;
	x += 1.3;
	x += 0.4;
	int y = x;
	x += y;
	x++;
	if (x < 5.65 || x > 5.75) {
		return 1;
	}

	if (0.6 + 0.7 < 1.2) {
		return 2;
	}

	x = 1.;
	if (x != 1) {
		return 3;
	}

	return 0;
}

int main36() {
	int ar[] = {1, 2, 3};

	if (ar == (void*)5) {
		return 1;
	}

	if (&ar[0] >= &ar[2]) {
		return 2;
	}

	return 0;
}

int main37_f2(int c, int b)
{
	return c - b;
}

int (*main37_f1(int a, int b))(int c, int b)
{
	if (a != b)
		return main37_f2;
	return 0;
}

int main37()
{
	int (* (*p)(int a, int b))(int c, int d) = main37_f1;


	return (*(*p)(0, 2))(2, 2);
}


int main38()
{
	int x;
	int y = x;
	struct { int a, b; } s;
	s.a = x;
	x = s.b;
	return 0;
}

int main39()
{
	int i, *q;
	void *p;

	i = i ? 0 : 0l;
	p = i ? (void *) 0 : 0;
	p = i ? 0 : (void *) 0;
	p = i ? 0 : (const void *) 0;
	q = i ? 0 : p;
	q = i ? p : 0;
	q = i ? q : 0;
	q = i ? 0 : q;

	return (int) q;
}

struct S40 { int a; int b; };
struct S40 *s40 = &(struct S40) { 1, 2 };

int
main40()
{
	if(s40->a != 1)
		return 1;
	if(s40->b != 2)
		return 2;
	return 0;
}

int main41()
{
	typedef enum { a, b, c } d;
	typedef enum { e = b, f, g = f + c } h;
	typedef enum { i = 6, j = g + i, k } m;

	if (j != 10) {
		return 1;
	}

	{
		int j = 2;
		{
			typedef enum { i = 10, j = g * i, k } m;
			if (j != 40) {
				return 2;
			}
		}
		if  (j != 2) {
			return 3;
		}
	}

	if (j != 10) {
		return 4;
	}

	return 0;
}

int main42()
{
	int x;
	long long l;
	
	x = 0;
	l = 0;
	
	x = ~x;
	if (x != 0xffffffff)
		return 1;
	
	l = ~l;
	if (x != 0xffffffffffffffff)
		return 2;

	return 0;
}

int main43() {
	unsigned int uint = 5;
	signed int sint = -3;

    if (-uint < 0) {
    //  ^^^^^ warning: this arithmetic operation will overflow
		return 1;
	}
	if ((sint << uint) >= 0) {
		return 2;
	}

	unsigned short ushort = 0xabcd;
	uint = 8;
	if ((ushort << uint) != 0xabcd00) {
		return 3;
	}

	long long ll = 12;
	uint = 0x12345678;

	if ((uint << ll) != 0x45678000) {
		return 4;
	}

	return 0;
}

static int i44 = 5;

int main44() {
	// The point of this test is to keep the do while at the start of function
	do {
		if (i44 == 0) {
			return i44;
		}
		i44 -= 1;
	} while(1);
}

int main45_aux(unsigned long input) {
	switch(input) {
		case 2:
			return 10;
		case 1:
			return 20;
		default:
			return 3;
	};
}

int main45() {
	return main45_aux(1) + 2 * main45_aux(2) + 3 * main45_aux(3) - 49;
}

int main46()
{
	if (offsetof(struct { int x; int y; int z; }, x) != 0)
		return 1;
	if (offsetof(struct { int x; int y; int z; }, y) != 4)
		return 2;
	if (offsetof(struct { int x; int y; int z; }, z) != 8)
		return 3;

	return 0;
}

int main47() {
	char x = sizeof(x);
	{
		int a = x, x = sizeof(x), b = x;
		if (x != sizeof(int) || a != sizeof(char) || b != x) {
			return 1;
		}
	}
	if (x != sizeof(char)) {
		return 2;
	}
	return 0;
}

/* Comma operator: in ternary then-branch, for-loop increment, standalone. */
int main48() {
	int a = 0, b = 0;
	/* comma in ternary then-branch (C grammar: ? expression : cond-expr) */
	int x = 1 ? (a = 10, b = 20, a + b) : -1;
	if (x != 30 || a != 10 || b != 20) return 1;
	/* comma in for init and increment */
	int i, j;
	for (i = 0, j = 10; i < 3; i++, j--) {}
	if (i != 3 || j != 7) return 2;
	return 0;
}

/* Unsized array of function pointers: size inferred from initializer count. */
static int add_one(int x) { return x + 1; }
static int add_two(int x) { return x + 2; }
static int add_three(int x) { return x + 3; }

int main49() {
	int (*fns[])(int) = { add_one, add_two, add_three };
	if (sizeof(fns) / sizeof(fns[0]) != 3) return 1;
	if (fns[0](10) != 11) return 2;
	if (fns[1](10) != 12) return 3;
	if (fns[2](10) != 13) return 4;
	return 0;
}

int main50() {
	static int primes[] = { 2, 3, 5, 7, 11 };
	if (sizeof(primes) / sizeof(primes[0]) != 5) return 1;
	if (primes[0] != 2) return 2;
	if (primes[4] != 11) return 3;
	primes[0] = 99;
	if (primes[0] != 99) return 4;
	return 0;
}

struct main51_s {
	char bytes[16];
};

static int main51_aux(struct main51_s *p) {
	char copy[sizeof(p->bytes)];
	return sizeof(copy) != sizeof(p->bytes);
}

typedef struct main51_typedef_s {
	char bytes[16];
} main51_typedef_s;

static int main51_typedef_aux(main51_typedef_s *p) {
	char copy[sizeof(p->bytes)];
	return sizeof(copy) != sizeof(p->bytes);
}

int main51() {
	struct main51_s s;
	if (main51_aux(&s)) {
		return 1;
	}
	main51_typedef_s s2;
	if (main51_typedef_aux(&s2)) {
		return 2;
	}
	return 0;
}

static unsigned char main52_data[] = { 1, 2, 3, 4, 5 };
static unsigned char *main52_ptr = &main52_data[2];

int main52() {
	if (main52_ptr[0] != 3) return 1;
	if (main52_ptr[1] != 4) return 2;
	return 0;
}

typedef int main53_i32;
struct main53_s {
	main53_i32 const *p;
};

int main53() {
	main53_i32 v = 7;
	struct main53_s s;
	s.p = &v;
	if (*s.p != 7) return 1;
	return 0;
}

int main54() {
	int ar[3] = {1, 2, 5};
	struct { int* a; int b[3]; } st = { ar, {2, 3, 6} };
	if (st.a[1] != 2 || st.b[1] != 3) {
		return 1;
	}
	return 0;
}

int main55() {
	struct Foo { int a; int b; } s1 = { 1, 2 }, s2 = { 3, 4 };
	struct Foo s3 = 1 ? s1 : s2;
	struct Foo s4 = 0 ? s1 : s2;
	if (s3.a != 1 || s3.b != 2 || s4.a != 3) {
		return 1;
	}
	return 0;
}

int main56() {
	float x = 1.5f;
	if (-x > -1.4f) {
		return 1;
	}
	if (-x < -1.6f) {
		return 2;
	}
	double y = 1.2f;
	if (y == 1.2) {
		return 3;
	}
	y = 1.5f;
	if (y != 1.5) {
		return 4;
	}
	return 0;
}

static int main57_expect_const_char(const char *p) {
	return p[0] != 'b';
}

int main57() {
	const char *z = "abc";
	return main57_expect_const_char(&z[1]);
}

int main58() {
	const void *p = 0 ? (const char *)"x" : (void *)1;
	return p != (void *)1;
}

struct main59_schema {
	int x;
};

int main59() {
	struct {
		union {
			struct main59_schema *schema;
			char *name;
		};
	} v;
	v.name = "abc";
	return v.name[1] != 'b';
}

extern long write(int, const void *buf, unsigned long count);

typedef void (*main60_syscall_ptr)();

int main60() {
	main60_syscall_ptr p = (main60_syscall_ptr)write;
	return p == 0;
}

int main61() {
	int *p = 0;
	if (!p) {
		return 0;
	}
	return 1;
}

int main62_helper(int x) {
	return x + 1;
}

int main62() {
	typedef int (*main62_fn1)(int);
	typedef int (*main62_fn2)(unsigned);
	main62_fn1 a = main62_helper;
	main62_fn2 b = (main62_fn2)a;
	return b(1) != main62_helper(1);
}

static const unsigned char main63_magic[] = { 1, 2, 3 };

int main63() {
	unsigned char buf[sizeof(main63_magic) + 1];
	return sizeof(buf) != 4;
}

static int main64_fn(int x) {
	return x + 1;
}

int main64() {
	return (1 ? main64_fn : 0)(1) != 2;
}

static int main65_fn(int x) {
	return x + 1;
}

int main65() {
	unsigned long long addr = (unsigned long long)main65_fn;
	return addr == 0;
}

typedef struct {
	int vals[3];
} main66_s;

int main66() {
	static const main66_s s = {{4, 5, 6}};
	const main66_s *p = &s;
	return p->vals[1] != 5;
}

typedef union {
	int x;
	int y;
} main67_u;

static int main67_flag = 1;

int main67() {
	switch (main67_flag) {
	main67_u u;
	case 1:
		u.x = 7;
		return u.x != 7;
	}
	return 0;
}

static int main68_fill(double *p) {
	*p = 3.5;
	return 0;
}

static int main68_flag = 1;

int main68() {
	if (main68_flag) goto main68_label;
	double r;
main68_label:
	if (main68_fill(&r)) return 1;
	return r != 3.5;
}

typedef int (*main69_fp)();

static int main69_a() {
	return 1;
}

static int main69_b() {
	return 2;
}

int main69() {
	main69_fp a = main69_a;
	main69_fp b = main69_b;
	return (a != a) || (a == b);
}

static const char main70_text[] = "co2";

const char *main70_get() {
	return main70_text;
}

int main70() {
	return main70_get()[1] != 'o';
}

static int main71_i;

int main71() {
	while (main71_i < 3 && main71_i >= 0) {
		main71_i += 1;
	}
	return main71_i != 3;
}

extern const char main72_ext[];
extern const char main72_decl_only[];
const char main72_ext[] = "ok";

int main72() {
	return main72_ext[1] != 'k';
}

; // This is to test if empty decl works

int main73() {
	const char *name = __func__;
	return name[0] != 'm' || name[4] != '7' || name[5] != '3';
}

int main74() {
	int r = 0;
	unsigned char a[sizeof(r)];
	return sizeof(a) != sizeof(int);
}

static int main75_expect_true(_Bool value) {
	return !value;
}

int main75() {
	int x = 0;
	return main75_expect_true(&x);
}

int main76_x = 0;

int
main76()
{
	switch(main76_x)
		case 0:
			;
	switch(main76_x)
		case 0:
			switch(main76_x) {
				case 0:
					goto next;
				default:
					return 1;
			}
	return 1;
	next:
	switch(main76_x)
		case 1:
			return 1;
	switch(main76_x) {
		{
			main76_x = 1 + 1;
			foo:
			case 1:
				return 1;
		}
	}
	switch(main76_x) {
		case 0:
			return main76_x;
		case 1:
			return 1;
		default:
			return 1;
	}
}

int main77() {
	int a = 2;
	int x = (int)(a == 2);
	_Bool b = x;
	int y = (int)b;
	int z = b;
	int p = (a != 2);
	if (p || !x || !b || !y || !z) {
		return 1;
	}
	return 0;
}

typedef int (main78_fp)();

static int main78_a() {
	return 1;
}

static int main78_b() {
	return 2;
}

int main78() {
	main78_fp* a = main78_a;
	main78_fp* b = main78_b;
	return (a != a) || (a == b) || (a() != 1) || (b() != 2);
}

struct main79_s { int a; };
typedef struct main79_s main79_st;

typedef int (main79_fp)(int, char*, struct main79_s, int[], struct main79_s*, struct main79_s[5], main79_st);

int main79_f(int a, char* b, struct main79_s c, int d[], struct main79_s* e, struct main79_s f[5], main79_st g) {
	return a + c.a + g.a;
}

int main79() {
	main79_fp *f = main79_f;
	return f(5, NULL, (struct main79_s) { 2 }, NULL, NULL, NULL, (struct main79_s) { 3 }) - 10;
}

typedef int main80_t1[3];
typedef int main80_t2[];

int main80_aux(int ar1[5], int ar2[static 5], int ar3[1 + 2], main80_t1 ar4, main80_t2 ar5, int ar6[], int ar7[const]) {
	return ar1[1] + ar2[2] + ar3[3] + ar4[4] + ar5[5] + ar6[6] + ar7[7];
}

int main80() {
	int ar[] = {0, 1, 2, 3, 4, 5, 6, 7};
	return main80_aux(ar, ar, ar, ar, ar, ar, ar) != 28;
}

int main81_aux() { return 0; }

int main81() {
	int v = 5;
	int *a = &v;
	int *b = &v;
	int *c = 0;

	if (!(a && b)) {
		return 1;
	}

	if (a && c) {
		return 2;
	}

	if (!(a || c)) {
		return 3;
	}

	if (!(a && !c)) {
		return 4;
	}

	int (*f)() = main81_aux;
	int (*g)() = 0;

	if (!main81_aux) {
		return 5;
	}
	if (!(f && main81_aux)) {
		return 6;
	}
	if (g && f) {
		return 7;
	}
	if (main81_aux != f) {
		return 8;
	}
	if (main81_aux == g) {
		return 9;
	}
	return 0;
}

int (main82_aux1)(int n) { return n; }
int (((((main82_aux2)))))() { return 0; }
static int (main82_aux3)(int n) { return n; }
static int8_t (main82_aux4)(int8_t n) { return n; }
static int8_t (main82_aux4)(int8_t n);

int main82() {
	if (main82_aux1(0)) {
		return 1;
	}
	if (main82_aux2()) {
		return 1;
	}
	if (main82_aux3(0)) {
		return 1;
	}
	if (main82_aux4(0)) {
		return 1;
	}
	return 0;
}

int main83() {
	typedef struct {
		int atom;
		int flags;
	} JSShapeProperty;

	static const JSShapeProperty props1[] = {
        {.atom = 1, .flags = 2},
    };
	static const JSShapeProperty props2[] = {
        {.atom = 1, .flags = 2},
        {.atom = 3, .flags = 4},
    };
    return props1[0].atom + props1[0].flags + props2[0].atom + props2[1].flags != 8;
}

typedef int T_84;
int sum_array_84(const T_84[5]);
int sum_array_84(const T_84 a[5]) {
	return a[0] + a[1] + a[2] + a[3] + a[4];
}
int main84() {
	T_84 arr[5] = {1, 2, 3, 4, 5};
	return sum_array_84(arr) != 15;
}

struct opaque_85;
static inline struct opaque_85 *cast_null_85(void) {
	return (struct opaque_85 *)0;
}
int main85() {
	(void)cast_null_85;
	return 0;
}

int main86() {
	int x = 0;
	int y = 0;
	for (;;) {
		switch (x) {
		case 0:
			y = 10;
			break;
		case 1:
			y += 5;
			break;
		default:
			return (y != 15);
		}
		x += 1;
	}
}

int fill_with_hello(char** out) {
	*out = "hello";
}

static void clobber_stack() {
  char buf[256];
  for (int i = 0; i < 256; i++)
    buf[i] = 'X';
}

int main87() {
	char* s = 0;
	if (s) {
		return 1;
	}
	fill_with_hello(&s);
	clobber_stack();
	if (!s) {
		return 1;
	}
	if (s[0] != 'h' || s[4] != 'o' || s[5] != 0) {
		return 1;
	}
	return 0;
}

typedef int (*main_ty)();

int main() {
	main_ty mains[] = {
		main0,
		main1, main2, main3, main4, main5,
		main6, main7, main8, main9, main10,
		main11, main12, main13, main14, main15,
		main16, main17, main18, main19, main20,
		main21, main22, main23, main24, main25,
		main26, main27, main28, main29, main30,
		main31, main32, main33, main34, main35,
		main36, main37, main38, main39, main40,
		main41, main42, main43, main44, main45,
		main46, main47, main48, main49, main50,
		main51, main52, main53, main54, main55,
		main56, main57, main58, main59, main60,
		main61, main62, main63, main64, main65,
		main66, main67, main68, main69, main70,
		main71, main72, main73, main74, main75,
		main76, main77, main78, main79, main80,
		main81, main82, main83, main84, main85,
		main86, main87,
	};
	
	int i;
	for (i = 0; i < sizeof(mains) / sizeof(mains[0]); i += 1) {
		if (mains[i]()) {
			return i;
		}
	}
	return 0;
}
