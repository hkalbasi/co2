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
	return 0;
}

int main() {
	if (main1() || main2() || main3()) {
		return 6;
	}
	return 0;
}
