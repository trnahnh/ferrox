_Z12latency_loopRN6ferrox8SpscRingIyE8ProducerERNS1_8ConsumerEy:
.LFB3806:
	pushq	%rsi
	pushq	%rbx
	movq	%rdx, %r9
	testq	%r8, %r8
	je	.L36
	xorl	%r10d, %r10d
	xorl	%r11d, %r11d
	jmp	.L35
.L30:
	movq	16(%rdx), %rsi
	movq	(%rdx), %rbx
	andq	%rax, %rsi
	addq	$1, %rax
	movq	%r10, (%rbx,%rsi,8)
	movq	%rax, 64(%rdx)
	movq	%rax, 16(%rcx)
	movq	16(%r9), %rax
	cmpq	24(%r9), %rax
	je	.L32
.L34:
	movq	(%r9), %rdx
	addq	$1, %r10
	movq	16(%rdx), %rsi
	movq	(%rdx), %rbx
	andq	%rax, %rsi
	addq	$1, %rax
	movq	(%rbx,%rsi,8), %rbx
	movq	%rax, 128(%rdx)
	addq	%rbx, %r11
	movq	%rax, 16(%r9)
	cmpq	%r8, %r10
	je	.L28
.L35:
	movq	16(%rcx), %rax
	movq	(%rcx), %rdx
	movq	%rax, %rbx
	subq	24(%rcx), %rbx
	cmpq	8(%rdx), %rbx
	jne	.L30
	movq	128(%rdx), %rbx
	movq	%rax, %rsi
	subq	%rbx, %rsi
	movq	%rbx, 24(%rcx)
	movq	(%rcx), %rdx
	cmpq	8(%rdx), %rsi
	jne	.L30
	movq	16(%r9), %rax
	cmpq	24(%r9), %rax
	jne	.L34
.L32:
	movq	(%r9), %rdx
	movq	64(%rdx), %rdx
	movq	%rdx, 24(%r9)
	cmpq	%rdx, %rax
	jne	.L34
	addq	$1, %r10
	cmpq	%r8, %r10
	jne	.L35
.L28:
	movq	%r11, %rax
	popq	%rbx
	popq	%rsi
	ret
.L36:
	xorl	%r11d, %r11d
	movq	%r11, %rax
	popq	%rbx
	popq	%rsi
	ret
