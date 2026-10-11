_ZN9asm_probe13latency_bench17h517c9ec7e6aad4abE:
.Lfunc_begin2:
	pushq	%rbp
	pushq	%r15
	pushq	%r14
	pushq	%rsi
	pushq	%rdi
	pushq	%rbx
	subq	$120, %rsp
	leaq	112(%rsp), %rbp
	movq	$-2, (%rbp)
	movq	%rcx, %rdi
	leaq	-72(%rbp), %rcx
	callq	_ZN9asm_probe4ring11ring_buffer17hca9e21352b6b5623E
	movq	-72(%rbp), %r14
	movq	-48(%rbp), %r15
	testq	%rdi, %rdi
	movq	%r14, -24(%rbp)
	je	.LBB13_1
	movq	-64(%rbp), %r8
	movq	-56(%rbp), %r10
	movq	-40(%rbp), %r9
	movq	-32(%rbp), %r11
	xorl	%esi, %esi
	xorl	%ebx, %ebx
	.p2align	4
.LBB13_8:
	leaq	(%r8,%rbx), %rax
	movq	%rax, %rcx
	subq	%r10, %rcx
	cmpq	192(%r14), %rcx
	jne	.LBB13_10
	movq	128(%r14), %r10
	movq	%rax, %rcx
	subq	%r10, %rcx
	cmpq	192(%r14), %rcx
	je	.LBB13_19
.LBB13_10:
	andq	200(%r14), %rax
	movq	216(%r14), %rdx
	cmpq	%rdx, %rax
	jae	.LBB13_11
	leaq	(%r9,%rbx), %rcx
	movq	208(%r14), %rdx
	movq	%rbx, (%rdx,%rax,8)
	leaq	(%r8,%rbx), %rax
	incq	%rax
	movq	%rax, 64(%r14)
	cmpq	%r11, %rcx
	jne	.LBB13_22
	movq	64(%r15), %rax
	cmpq	%rax, %r11
	movq	%rax, %r11
	je	.LBB13_24
.LBB13_22:
	andq	200(%r15), %rcx
	movq	216(%r15), %rdx
	cmpq	%rdx, %rcx
	jae	.LBB13_23
	movq	208(%r15), %rax
	leaq	(%r9,%rbx), %rdx
	incq	%rdx
	incq	%rbx
	addq	(%rax,%rcx,8), %rsi
	movq	%rdx, 128(%r15)
	cmpq	%rbx, %rdi
	jne	.LBB13_8
	movq	%r15, -16(%rbp)
	lock		decq	(%r15)
	je	.LBB13_3
	jmp	.LBB13_4
.LBB13_1:
	xorl	%esi, %esi
	movq	%r15, -16(%rbp)
	lock		decq	(%r15)
	jne	.LBB13_4
.LBB13_3:
	#MEMBARRIER
.Ltmp12:
	leaq	-16(%rbp), %rcx
	callq	_ZN5alloc4sync16Arc$LT$T$C$A$GT$9drop_slow17he116ba51d21f7f94E
.Ltmp13:
.LBB13_4:
	movq	-24(%rbp), %rax
	movq	%rax, -72(%rbp)
	lock		decq	(%rax)
	jne	.LBB13_6
	#MEMBARRIER
	leaq	-72(%rbp), %rcx
	callq	_ZN5alloc4sync16Arc$LT$T$C$A$GT$9drop_slow17he116ba51d21f7f94E
.LBB13_6:
	movq	%rsi, %rax
	addq	$120, %rsp
	popq	%rbx
	popq	%rdi
	popq	%rsi
	popq	%r14
	popq	%r15
	popq	%rbp
	retq
.LBB13_23:
.Ltmp6:
	movq	%r15, -8(%rbp)
	leaq	anon.546c0b00915c432f789238a9c7599248.15(%rip), %r8
	callq	_ZN4core9panicking18panic_bounds_check17ha95d63c16a71bfedE
.Ltmp7:
	jmp	.LBB13_12
.LBB13_11:
.Ltmp4:
	movq	%r15, -8(%rbp)
	leaq	anon.546c0b00915c432f789238a9c7599248.15(%rip), %r8
	movq	%rax, %rcx
	callq	_ZN4core9panicking18panic_bounds_check17ha95d63c16a71bfedE
.Ltmp5:
	jmp	.LBB13_12
.LBB13_19:
	movq	%r15, -8(%rbp)
	movq	%rbx, -16(%rbp)
.Ltmp10:
	leaq	anon.546c0b00915c432f789238a9c7599248.11(%rip), %rax
	movq	%rax, 32(%rsp)
	leaq	anon.546c0b00915c432f789238a9c7599248.2(%rip), %rcx
	leaq	anon.546c0b00915c432f789238a9c7599248.1(%rip), %r9
	leaq	-16(%rbp), %r8
	movl	$43, %edx
	callq	_ZN4core6result13unwrap_failed17h619e9520647c143eE
.Ltmp11:
	jmp	.LBB13_12
.LBB13_24:
.Ltmp8:
	movq	%r15, -8(%rbp)
	leaq	anon.546c0b00915c432f789238a9c7599248.12(%rip), %rax
	movq	%rax, 32(%rsp)
	leaq	anon.546c0b00915c432f789238a9c7599248.2(%rip), %rcx
	leaq	anon.546c0b00915c432f789238a9c7599248.3(%rip), %r9
	leaq	-16(%rbp), %r8
	movl	$43, %edx
	callq	_ZN4core6result13unwrap_failed17h619e9520647c143eE
.Ltmp9:
.LBB13_12:
	ud2
	.long	$cppxdata$_ZN9asm_probe13latency_bench17h517c9ec7e6aad4abE@IMGREL
	.section	.text,"xr",one_only,_ZN9asm_probe13latency_bench17h517c9ec7e6aad4abE,unique,13
