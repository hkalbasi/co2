//@ mode: c
//@ run-status: 8

typedef int name1;

int main()
{
	int x = 2;
	int y = 0;
	int name2 = 3;
	{
		name2 * x;
		y += name2;
		y += x;
		name1 * x;
		x = &name2;
		y += *x;
		return y;
	}
	return 5;
}
