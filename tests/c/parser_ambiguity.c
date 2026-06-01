//@ mode: c
//@ run-status: 0

typedef int name1;

int context_sensitive_names()
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

int turbo_fish() {
	int a = 2, b = 5, c = 3, d = 7;
	if (a < b > + c) {
		return 1;
	}
	return 0;
}

int main() {
	if (context_sensitive_names() != 8) {
		return 1;
	}
	if (turbo_fish()) {
		return 2;
	}
	return 0;
}