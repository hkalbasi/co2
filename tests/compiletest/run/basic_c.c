//@ mode: c
//@ run-status: 0

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

int main() {
	if (main1() || main2()) {
		return 6;
	}
	return 0;
}
