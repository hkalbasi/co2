//@ mode: c
//@ run-status: 0

#include <stdint.h>

#define FOO 0
#define BAR 5
#define ADD2(x) ((x) + 2)

#if BAR == 5
#define SELECTED 1
#else
#define SELECTED 0
#endif

#ifdef BAR
#define HAD_BAR 1
#else
#define HAD_BAR 0
#endif

#undef BAR

#ifdef BAR
#define BAR_GONE 0
#else
#define BAR_GONE 1
#endif

int main()
{
	int32_t x;

	x = FOO;
	x = x + ADD2(3);
	x = x + SELECTED;
	x = x + HAD_BAR;
	x = x + BAR_GONE;

	return x - 8;
}
