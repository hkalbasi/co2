//@ mode: c
//@ run-status: 0

typedef int name1;

int main()
{
	int x = 2;
	int name2 = 3;
	{
		name2 * x;
		name1 * x;
		x = &name2;
		return *x - 3;
	}
	return 5;
}
