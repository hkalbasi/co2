//@ mode: c
//@ run-status: 0

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

	return 0;
}

int main15()
{
	int a[5] = { [2] = 7, [0] = 9, 1 };
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
	if (s.x - 3 || s.y || s.z - 8)
		return 1;
	if (p[0].x - 1 || p[0].y - 2 || p[1].x - 5 || p[1].y - 6)
		return 1;
	if (m[0][1] - 3 || m[1][2] - 4)
		return 1;
	if (m[0][0] || m[0][2] || m[1][0] || m[1][1])
		return 1;

	return 0;
}

enum Color { RED, GREEN = 5, BLUE, WHITE = BLUE + 3 };

int main16()
{
	enum Color c;
	int x;

	c = BLUE;
	if (c - 6)
		return 1;
	if (RED)
		return 1;
	if (GREEN - 5)
		return 1;
	if (WHITE - 9)
		return 1;

	x = RED + GREEN + BLUE + WHITE;
	if (x - 20)
		return 1;

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

	return 0;
}

void do_complex(int *p)
{
	*p = *p + 1;
}

int main22()
{
	int side_effect_counter;
	int x;

	side_effect_counter = 0;
	x = ({ int local; local = 4; do_complex(&side_effect_counter); local + 1; });
	if (x - 5)
		return 1;
	if (side_effect_counter - 1)
		return 1;

	return 0;
}

int counter = 0;
int uninit;

int count() {
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
// unsized_int_array static_ar1 = {10, 20, 3, 15, 1000, 60, 16};
// char static_ar2[] = "foo bar foobar";

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

	char ar3[] = "Hello world";
	if (ar3[0] != 'H' || ar3[10] != 'd' || sizeof(ar3) != 12) {
		return 3;
	}

	int ar4[][5] = {1, 2, 3, 4, 5, 6, 7};

	if (ar4[1][1] != 7 || ar4[0][4] != 5 || sizeof(ar4) / sizeof(int) != 10) {
		return 3;
	}
	
	// if (static_ar1[0] != 10 || static_ar1[6] != 16 || sizeof(static_ar1) / sizeof(int) != 7) {
	// 	return 4;
	// }

	// if (static_ar2[0] != 'f' || static_ar2[14] != 0 || sizeof(static_ar2) != 15) {
	// 	return 5;
	// }

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

int main32()
{
	count_inner_static();
	count_inner_static();
	return count_inner_static() - 3;
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

int main39(void)
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
		main41,
	};
	
	int i;
	for (i = 0; i < sizeof(mains) / sizeof(mains[0]); i += 1) {
		if (mains[i]()) {
			return i;
		}
	}
	return 0;
}
