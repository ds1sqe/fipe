#BNF
##RULES
    RootNode(aka Program) = Statement | Statement* ;

    Statement = Let_Statement | Expression_Statement | Block_Statement | Return_Statement ;

      Let_Statement = Token_let Identifier Token_Assignment Expression ;
        ex) let foo = 20;


      Expression_Statement = Expression (;) ;
        ex) 20; , 20 + 30; , call_fn(foo)


      Block_Statement = { Statement* } ;


      Return_Statement = return (Expression) ;


    Expression = Identifier_Expression | Infix_Expression | Prefix_Expression 
                                       | Call_Expression | Index_Expression 
                                       | If_Expression | Literal_Expression  ;


      Identifier_Expression = Identifier ;


      Infix_Expression = Expression Infix_Operator Expression
        ex) 1 + 2, a - b, big >= small, ... 


      Prefix_Expression = Prefix_Operator Expression
        ex) 1 + 2, a - b, big >= small, ... 

      
      Call_Expression = Identifier ( Expression* ) | Function_Literal ( Expression* ) 
        ex) some_fn(arg), another() , fn { return foo; }

      If_Expression = if ( Expression ) Block_Statement ( else Block_Statement )
        ex) if (flag == true) { return foo }
        ex) if (flag == true) { foo } else { return 20; }


      Index_Expression = Identifier [ Expression ] | Array_Literal  [ Expression ]


      Literal_Expression = Integer_Literal | Boolean_Literal | String_Literal | Function_Literal | Array_Literal

      
        Integer_Literal = (-) [ 0..=9 ] * ;


        String_Literal = "( [a-z|A-Z|0-9] * )" ;


        Boolean_Literal = true | false ;


        Function_Literal = fn (Identifier) ( (Identifier*) ) Block_Statement ;


        Array_Literal = [Expression (,Expression)* ]

