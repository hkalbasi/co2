//@ mode: c
//@ run-status: 0

/* Jumps across GNU statement-expression boundaries: goto out of a block,
   break/continue (inner and outer loops), switch/case. Labels are unique
   per function since C labels are function-scoped.
   NOTE: gcc rejects jumping *into* a statement expression
   ("jump into statement expression"); CO2 currently accepts it. Not
   covered here. */

// A. goto out of a stmt-expr (skips the assignment and the next store).
int goto_out()
{
	int r = 7;
	r = ({ goto doneA; r = 99; 1; });
	r = 100;		// skipped
doneA:	r += 5;		// r still 7 -> 12
	return r - 12;
}

// A2. goto out forward to a later function-level label (skips both stores).
int goto_out_forward()
{
	int s = 0;
	s = ({ goto outS; 1; });
	s = 100;		// skipped
outS:	s += 2;		// s still 0 -> 2
	return s - 2;
}

// C1. break inside a loop inside a stmt-expr.
int break_inner()
{
	int r = ({ int s = 0; for (int i = 0; i < 5; i++) { if (i == 3) break; s += i; } s; });
	return r - 3;
}

// C2. continue inside a loop inside a stmt-expr.
int continue_inner()
{
	int r = ({ int s = 0; for (int i = 0; i < 5; i++) { if (i < 3) continue; s += i; } s; });
	return r - 7;
}

// C3. while(1) + continue/break inside a stmt-expr.
int while_inner()
{
	int r = ({ int i = 0; int s = 0; while (1) { i++; if (i < 3) continue; if (i > 5) break; s += i; } s; });
	return r - (3 + 4 + 5);
}

// D. stmt-expr inside a loop; outer break/continue unaffected.
int expr_in_loop()
{
	int s = 0;
	for (int i = 0; i < 10; i++) {
		int v = ({ i * 2; });
		if (v == 4)
			continue;
		if (v == 8)
			break;
		s += v;
	}
	return s - (0 + 2 + 6);
}

// D2. break directly inside a stmt-expr exits the outer loop.
int break_outer()
{
	int s = 0, k = 0;
	for (int i = 0; i < 10; i++) {
		int v = ({ if (i == 2) break; i * 10; });
		s += v;
		k++;
	}
	if (s - 10)
		return 1;
	return k - 2;
}

// D3. continue directly inside a stmt-expr continues the outer loop.
int continue_outer()
{
	int s = 0;
	for (int i = 0; i < 5; i++) {
		({ if (i == 2) continue; s += i; });
	}
	return s - (0 + 1 + 3 + 4);
}

// E. switch inside a stmt-expr.
int switch_inner()
{
	int r = ({ int x = 2; int y = 0; switch (x) { case 1: y = 10; break; case 2: y = 20; break; default: y = 30; } y; });
	return r - 20;
}

// E2. fallthrough and default inside a stmt-expr.
int switch_fallthrough()
{
	int r = ({ int x = 1; int y = 0; switch (x) { case 1: y += 10; case 2: y += 20; break; default: y += 30; } y; });
	if (r - 30)
		return 1;
	int r2 = ({ int x = 9; int y = 0; switch (x) { case 1: y += 10; break; default: y += 30; } y; });
	return r2 - 30;
}

// H. break in switch in stmt-expr in loop: breaks only the switch.
int break_switch_only()
{
	int s = 0;
	for (int i = 1; i <= 3; i++) {
		int v = ({ int t = 0; switch (i) { case 2: t = 50; break; default: t = i; break; } t + 1; });
		s += v;
	}
	return s - (2 + 51 + 4);
}

// N. nested stmt-expr with an inner goto.
int nested()
{
	int r = ({ int a = ({ goto innerN; 1; innerN: 2; }); a + 10; });
	return r - 12;
}

// N2. goto from an inner stmt-expr to an outer stmt-expr label.
int nested_outward()
{
	int r = ({ int a = 5; int b = ({ goto midN2; 100; }); goto doneN2; midN2: b = 20; a += b; a; doneN2: a + 1; });
	return r - 26;
}

// G. stmt-expr as a switch discriminant.
int discriminant()
{
	int w = 0;
	switch (({ int q = 2; q * 3; })) {
	case 6: w = 1; break;
	default: w = 2; break;
	}
	return w - 1;
}

// J. several labels inside one stmt-expr, jumping between them.
int multi_label()
{
	int r = ({ int v = 1; goto secondJ; firstJ: v += 10; goto doneJ; secondJ: v += 100; goto firstJ; doneJ: v; });
	return r - 111;
}

int main()
{
	if (goto_out())
		return 1;
	if (goto_out_forward())
		return 2;
	if (break_inner())
		return 3;
	if (continue_inner())
		return 4;
	if (while_inner())
		return 5;
	if (expr_in_loop())
		return 6;
	if (break_outer())
		return 7;
	if (continue_outer())
		return 8;
	if (switch_inner())
		return 9;
	if (switch_fallthrough())
		return 10;
	if (break_switch_only())
		return 11;
	if (nested())
		return 12;
	if (nested_outward())
		return 13;
	if (discriminant())
		return 14;
	if (multi_label())
		return 15;
	return 0;
}
