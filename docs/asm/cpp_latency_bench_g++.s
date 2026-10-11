_Z13latency_benchy:
.LFB3807:
	pushq	%rbp
	pushq	%rdi
	pushq	%rsi
	pushq	%rbx
	subq	$40, %rsp
	movl	$64, %edx
	movq	%rcx, %rbp
	movl	$256, %ecx
.LEHB0:
	call	_ZnwySt11align_val_t
.LEHE0:
	movdqu	.LC1(%rip), %xmm0
	movl	$8192, %ecx
	movq	%rax, %rbx
	leaq	8(%rax), %rdi
	movq	.LC0(%rip), %rax
	movq	$0, 64(%rbx)
	movq	%rax, 8(%rbx)
	leaq	16+_ZTVSt23_Sp_counted_ptr_inplaceIN6ferrox8SpscRingIyE5InnerESaIvELN9__gnu_cxx12_Lock_policyE2EE(%rip), %rax
	movq	%rax, (%rbx)
	movq	$0, 128(%rbx)
	movq	$0, 192(%rbx)
	movups	%xmm0, 72(%rbx)
.LEHB1:
	call	_Znay
.LEHE1:
	movq	64(%rbx), %rcx
	movq	%rax, 64(%rbx)
	testq	%rcx, %rcx
	je	.L61
	call	_ZdaPv
.L61:
	lock addl	$1, 8(%rbx)
	testq	%rbp, %rbp
	je	.L74
	xorl	%r11d, %r11d
	xorl	%edx, %edx
	xorl	%r10d, %r10d
	xorl	%eax, %eax
	xorl	%ecx, %ecx
	xorl	%esi, %esi
	jmp	.L69
.L65:
	movq	80(%rbx), %r9
	movq	64(%rbx), %r8
	andq	%rax, %r9
	addq	$1, %rax
	movq	%rcx, (%r8,%r9,8)
	movq	%rax, 128(%rbx)
	cmpq	%rdx, %r11
	je	.L92
.L67:
	movq	80(%rbx), %r9
	movq	64(%rbx), %r8
	addq	$1, %rcx
	andq	%rdx, %r9
	addq	$1, %rdx
	movq	(%r8,%r9,8), %r8
	movq	%rdx, 192(%rbx)
	addq	%r8, %rsi
	cmpq	%rbp, %rcx
	je	.L64
.L69:
	movq	%rax, %r8
	subq	%r10, %r8
	cmpq	72(%rbx), %r8
	jne	.L65
	movq	192(%rbx), %r10
	movq	%rax, %r8
	subq	%r10, %r8
	cmpq	72(%rbx), %r8
	jne	.L65
	cmpq	%rdx, %r11
	jne	.L67
.L92:
	movq	128(%rbx), %r11
	cmpq	%rdx, %r11
	jne	.L67
	addq	$1, %rcx
	cmpq	%rbp, %rcx
	jne	.L69
.L64:
	movabsq	$4294967297, %rax
	movq	8(%rbx), %rdx
	cmpq	%rax, %rdx
	je	.L93
	lock subl	$1, (%rdi)
	je	.L94
.L71:
	movabsq	$4294967297, %rax
	movq	8(%rbx), %rdx
	cmpq	%rax, %rdx
	je	.L95
	lock subl	$1, (%rdi)
	je	.L96
.L60:
	movq	%rsi, %rax
	addq	$40, %rsp
	popq	%rbx
	popq	%rsi
	popq	%rdi
	popq	%rbp
	ret
.L95:
	movq	(%rbx), %rax
	movq	%rbx, %rcx
	movq	$0, 8(%rbx)
	call	*16(%rax)
	movq	(%rbx), %rax
	movq	%rbx, %rcx
	call	*24(%rax)
	movq	%rsi, %rax
	addq	$40, %rsp
	popq	%rbx
	popq	%rsi
	popq	%rdi
	popq	%rbp
	ret
.L93:
	movq	(%rbx), %rax
	movq	%rbx, %rcx
	movq	$0, 8(%rbx)
	call	*16(%rax)
	movq	(%rbx), %rax
	movq	%rbx, %rcx
	call	*24(%rax)
	jmp	.L71
.L74:
	xorl	%esi, %esi
	jmp	.L64
.L94:
	movq	%rbx, %rcx
	call	_ZNSt16_Sp_counted_baseILN9__gnu_cxx12_Lock_policyE2EE24_M_release_last_use_coldEv
	jmp	.L71
.L96:
	movq	%rbx, %rcx
	call	_ZNSt16_Sp_counted_baseILN9__gnu_cxx12_Lock_policyE2EE24_M_release_last_use_coldEv
	jmp	.L60
.L76:
	movq	64(%rbx), %rcx
	movq	%rax, %rsi
	testq	%rcx, %rcx
	je	.L63
	call	_ZdaPv
.L63:
	movq	%rbx, %rcx
	movl	$64, %r8d
	movl	$256, %edx
	call	_ZdlPvySt11align_val_t
	movq	%rsi, %rcx
.LEHB2:
	call	_Unwind_Resume
	nop
.LEHE2:
.LLSDA3807:
	.byte	0xff
	.byte	0xff
	.byte	0x1
.LLSDACSB3807:
.LLSDACSE3807:
	.text
