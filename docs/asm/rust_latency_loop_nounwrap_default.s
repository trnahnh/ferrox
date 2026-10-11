_ZN9asm_probe21latency_loop_nounwrap17h2f889e88073a1aabE:
	pushq	%rsi
	pushq	%rdi
	subq	$40, %rsp
	xorl	%eax, %eax
	testq	%r8, %r8
	je	.LBB15_13
	xorl	%r11d, %r11d
	jmp	.LBB15_2
	.p2align	4
.LBB15_8:
	xorl	%r9d, %r9d
	addq	%r9, %rax
	incq	%r11
	cmpq	%r11, %r8
	je	.LBB15_13
.LBB15_2:
	movq	(%rcx), %rdi
	movq	8(%rcx), %rsi
	movq	%rsi, %r9
	subq	16(%rcx), %r9
	cmpq	192(%rdi), %r9
	jne	.LBB15_4
	movq	128(%rdi), %r9
	movq	%r9, 16(%rcx)
	movq	%rsi, %r10
	subq	%r9, %r10
	movq	(%rcx), %rdi
	cmpq	192(%rdi), %r10
	je	.LBB15_6
.LBB15_4:
	movq	200(%rdi), %r9
	andq	%rsi, %r9
	movq	216(%rdi), %r10
	cmpq	%r10, %r9
	jae	.LBB15_14
	movq	208(%rdi), %r10
	movq	%r11, (%r10,%r9,8)
	incq	%rsi
	movq	%rsi, 64(%rdi)
	movq	%rsi, 8(%rcx)
.LBB15_6:
	movq	(%rdx), %rdi
	movq	8(%rdx), %rsi
	cmpq	16(%rdx), %rsi
	jne	.LBB15_10
	movq	64(%rdi), %r9
	movq	%r9, 16(%rdx)
	cmpq	%r9, %rsi
	je	.LBB15_8
	movq	(%rdx), %rdi
.LBB15_10:
	movq	200(%rdi), %r9
	andq	%rsi, %r9
	movq	216(%rdi), %r10
	cmpq	%r10, %r9
	jae	.LBB15_14
	movq	208(%rdi), %r10
	movq	(%r10,%r9,8), %r9
	incq	%rsi
	movq	%rsi, 128(%rdi)
	movq	%rsi, 8(%rdx)
	addq	%r9, %rax
	incq	%r11
	cmpq	%r11, %r8
	jne	.LBB15_2
.LBB15_13:
	addq	$40, %rsp
	popq	%rdi
	popq	%rsi
	retq
.LBB15_14:
	leaq	anon.546c0b00915c432f789238a9c7599248.15(%rip), %r8
	movq	%r9, %rcx
	movq	%r10, %rdx
	callq	_ZN4core9panicking18panic_bounds_check17ha95d63c16a71bfedE
	int3
