_Z10probe_pushRN6ferrox8SpscRingIyE8ProducerEy:
	movq	16(%rcx), %rax
	movq	(%rcx), %r8
	movq	%rax, %r9
	subq	24(%rcx), %r9
	cmpq	8(%r8), %r9
	je	.L23
.L20:
	movq	16(%r8), %r10
	movq	(%r8), %r9
	andq	%rax, %r10
	addq	$1, %rax
	movq	%rdx, (%r9,%r10,8)
	movq	%rax, %rdx
	xchgq	64(%r8), %rdx
	movl	$1, %r9d
	movq	%rax, 16(%rcx)
.L19:
	movl	%r9d, %eax
	ret
.L23:
	movq	%rax, %r10
	movq	128(%r8), %r9
	subq	%r9, %r10
	movq	%r9, 24(%rcx)
	xorl	%r9d, %r9d
	movq	(%rcx), %r8
	cmpq	8(%r8), %r10
	jne	.L20
	jmp	.L19

_Z9probe_popRN6ferrox8SpscRingIyE8ConsumerE:
	movq	16(%rcx), %rax
	cmpq	24(%rcx), %rax
	je	.L25
.L27:
	movq	(%rcx), %rdx
	movq	16(%rdx), %r9
	movq	(%rdx), %r8
	andq	%rax, %r9
	addq	$1, %rax
	movq	(%r8,%r9,8), %r8
	movq	%rax, %r10
	xchgq	128(%rdx), %r10
	movq	%rax, 16(%rcx)
.L24:
	movq	%r8, %rax
	ret
.L25:
	movq	(%rcx), %rdx
	movq	64(%rdx), %rdx
	movq	%rdx, 24(%rcx)
	cmpq	%rdx, %rax
	jne	.L27
	xorl	%r8d, %r8d
	jmp	.L24
