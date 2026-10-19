
/vol/md1/tika-indra-artifacts/frozen-wp1-ad24c12-20261019/baseline/server-acceptance/indra-server:     file format elf64-x86-64


Disassembly of section .text:

00000000008e4e5d <.text+0x239d>:
  8e4e5d:	03 48 89             	add    -0x77(%rax),%ecx
  8e4e60:	46 08 48 89          	rex.RX or %r9b,-0x77(%rax)
  8e4e64:	4e 10 48 89          	rex.WRX adc %r9b,-0x77(%rax)
  8e4e68:	46 18 48 8d          	rex.RX sbb %r9b,-0x73(%rax)
  8e4e6c:	3d 8e 32 8b ff       	cmp    $0xff8b328e,%eax
  8e4e71:	48 8d 15 80 0d f7 02 	lea    0x2f70d80(%rip),%rdx        # 0x3855bf8
  8e4e78:	ff 15 fa 0f 03 03    	call   *0x3030ffa(%rip)        # 0x3915e78
  8e4e7e:	55                   	push   %rbp
  8e4e7f:	48 89 e5             	mov    %rsp,%rbp
  8e4e82:	48 83 ec 30          	sub    $0x30,%rsp
  8e4e86:	48 8d 45 f8          	lea    -0x8(%rbp),%rax
  8e4e8a:	48 89 38             	mov    %rdi,(%rax)
  8e4e8d:	48 8d 4d f0          	lea    -0x10(%rbp),%rcx
  8e4e91:	48 89 31             	mov    %rsi,(%rcx)
  8e4e94:	48 8d 75 d0          	lea    -0x30(%rbp),%rsi
  8e4e98:	48 89 06             	mov    %rax,(%rsi)
  8e4e9b:	48 8b 05 06 10 03 03 	mov    0x3031006(%rip),%rax        # 0x3915ea8
  8e4ea2:	48 89 46 08          	mov    %rax,0x8(%rsi)
  8e4ea6:	48 89 4e 10          	mov    %rcx,0x10(%rsi)
  8e4eaa:	48                   	rex.W
  8e4eab:	89                   	.byte 0x89
  8e4eac:	46                   	rex.RX
