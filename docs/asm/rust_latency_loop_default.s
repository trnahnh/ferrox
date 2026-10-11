_ZN9asm_probe12latency_loop17hee4decf7763fe79fE:
	pushq	%rsi
	pushq	%rdi
	subq	$56, %rsp
	xorl	%eax, %eax
	testq	%r8, %r8
	je	.LBB12_12
	xorl	%r11d, %r11d
	.p2align	4
.LBB12_2:
	movq	(%rcx), %rdi
	movq	8(%rcx), %rsi
	movq	%rsi, %r9
	subq	16(%rcx), %r9
	cmpq	192(%rdi), %r9
	jne	.LBB12_4
	movq	128(%rdi), %r9
	movq	%r9, 16(%rcx)
	movq	%rsi, %r10
	subq	%r9, %r10
	movq	(%rcx), %rdi
	cmpq	192(%rdi), %r10
	je	.LBB12_6
.LBB12_4:
	movq	200(%rdi), %r9
	andq	%rsi, %r9
	movq	216(%rdi), %r10
	cmpq	%r10, %r9
	jae	.LBB12_5
	movq	208(%rdi), %r10
	movq	%r11, (%r10,%r9,8)
	incq	%rsi
	movq	%rsi, 64(%rdi)
	movq	%rsi, 8(%rcx)
	movq	(%rdx), %rdi
	movq	8(%rdx), %rsi
	cmpq	16(%rdx), %rsi
	jne	.LBB12_10
	movq	64(%rdi), %r9
	movq	%r9, 16(%rdx)
	cmpq	%r9, %rsi
	je	.LBB12_13
	movq	(%rdx), %rdi
.LBB12_10:
	movq	200(%rdi), %r9
	andq	%rsi, %r9
	movq	216(%rdi), %r10
	cmpq	%r10, %r9
	jae	.LBB12_5
	incq	%r11
	movq	208(%rdi), %r10
	addq	(%r10,%r9,8), %rax
	incq	%rsi
	movq	%rsi, 128(%rdi)
	movq	%rsi, 8(%rdx)
	cmpq	%r11, %r8
	jne	.LBB12_2
.LBB12_12:
	addq	$56, %rsp
	popq	%rdi
	popq	%rsi
	retq
.LBB12_5:
	leaq	anon.546c0b00915c432f789238a9c7599248.15(%rip), %r8
	movq	%r9, %rcx
	movq	%r10, %rdx
	callq	_ZN4core9panicking18panic_bounds_check17ha95d63c16a71bfedE
.LBB12_6:
	movq	%r11, 48(%rsp)
	leaq	anon.546c0b00915c432f789238a9c7599248.9(%rip), %rax
	movq	%rax, 32(%rsp)
	leaq	anon.546c0b00915c432f789238a9c7599248.2(%rip), %rcx
	leaq	anon.546c0b00915c432f789238a9c7599248.1(%rip), %r9
	leaq	48(%rsp), %r8
	movl	$43, %edx
	callq	_ZN4core6result13unwrap_failed17h619e9520647c143eE
.LBB12_13:
	leaq	anon.546c0b00915c432f789238a9c7599248.10(%rip), %rax
	movq	%rax, 32(%rsp)
	leaq	anon.546c0b00915c432f789238a9c7599248.2(%rip), %rcx
	leaq	anon.546c0b00915c432f789238a9c7599248.3(%rip), %r9
	leaq	47(%rsp), %r8
	movl	$43, %edx
	callq	_ZN4core6result13unwrap_failed17h619e9520647c143eE
	int3
