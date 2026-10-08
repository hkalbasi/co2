//@ mode: c
//@ run-status: 0

/* A trailing `label: expr;` still yields the expression's value (GNU C),
   and `goto label` from earlier must land on its evaluation. */
int taken()
{
	int q = 2;
	int r = ({ if (q) goto out; q = 99; out: q * 10; });
	if (r - 20)
		return 1;
	return 0;
}

int fallthrough()
{
	int q = 0;
	int r = ({ if (q) goto out; q = 99; out: q * 10; });
	if (r - 990)
		return 1;
	return 0;
}

int stacked()
{
	int v = 3;
	int r = ({ la: lb: v + 4; });
	if (r - 7)
		return 1;
	return 0;
}

int jump_to_stacked()
{
	int v = 100;
	int r = ({ goto lb; v = 200; la: lb: v + 1; });
	if (r - 101)
		return 1;
	return 0;
}

int jump_forward()
{
	int v = 1;
	int r = ({ goto out; v = 2; out: v + 40; });
	if (r - 41)
		return 1;
	return 0;
}

int void_tail()
{
	int flag = 0;
	({ if (flag) goto done; flag = 7; done: (void)0; });
	if (flag - 7)
		return 1;
	int n = 0;
	({ goto end; n = 111; end: (void)0; });
	if (n)
		return 1;
	return 0;
}

int main()
{
	if (taken())
		return 1;
	if (fallthrough())
		return 2;
	if (stacked())
		return 3;
	if (jump_to_stacked())
		return 4;
	if (jump_forward())
		return 5;
	if (void_tail())
		return 6;
	return 0;
}
