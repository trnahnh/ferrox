_ZN9asm_probe8push_u6417h4ff63959bebb5d3eE:
	subq	$40, %rsp
	movq	(%rcx), %r10
	movq	8(%rcx), %r8
	movq	%r8, %rax
	subq	16(%rcx), %rax
	cmpq	192(%r10), %rax
	jne	.LBB19_4
	movq	128(%r10), %rax
	movq	%rax, 16(%rcx)
	movq	%r8, %r9
	subq	%rax, %r9
	movq	(%rcx), %r10
	cmpq	192(%r10), %r9
	jne	.LBB19_4
	xorl	%eax, %eax
	addq	$40, %rsp
	retq
	movq	200(%r10), %rax
	andq	%r8, %rax
	movq	216(%r10), %r9
	cmpq	%r9, %rax
	jae	.LBB19_6
	movq	208(%r10), %r9
	movq	%rdx, (%r9,%rax,8)
	incq	%r8
	movq	%r8, 64(%r10)
	movq	%r8, 8(%rcx)
	movb	$1, %al
	addq	$40, %rsp
	retq
	leaq	anon.546c0b00915c432f789238a9c7599248.15(%rip), %r8
	movq	%rax, %rcx
	movq	%r9, %rdx
	callq	_ZN4core9panicking18panic_bounds_check17ha95d63c16a71bfedE
	int3

_ZN9asm_probe7pop_u6417hf93fb602eb891084E:
	subq	$40, %rsp
	movq	8(%rcx), %r8
	cmpq	16(%rcx), %r8
	jne	.LBB18_3
	movq	(%rcx), %rax
	movq	64(%rax), %rax
	movq	%rax, 16(%rcx)
	cmpq	%rax, %r8
	jne	.LBB18_3
	xorl	%eax, %eax
	addq	$40, %rsp
	retq
	movq	(%rcx), %r9
	movq	200(%r9), %rax
	andq	%r8, %rax
	movq	216(%r9), %rdx
	cmpq	%rdx, %rax
	jae	.LBB18_6
	movq	208(%r9), %rdx
	movq	(%rdx,%rax,8), %rax
	incq	%r8
	movq	%r8, 128(%r9)
	movq	%r8, 8(%rcx)
	addq	$40, %rsp
	retq
	leaq	anon.546c0b00915c432f789238a9c7599248.15(%rip), %r8
	movq	%rax, %rcx
	callq	_ZN4core9panicking18panic_bounds_check17ha95d63c16a71bfedE
	int3
